use crate::error::{FlupError, Result};
use crate::storage::fs_util::read_optional;
use crate::upgrade::template::{
    ConflictPreview, FileDecision, Report, TemplateParameters, decide_file,
};
use crate::upgrade::text_merge::diff_preview;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const ROOT_TEMPLATE_FILES: [&str; 2] = ["analysis_options.yaml", ".gitignore"];
const NON_TEMPLATE_ENTRIES: [&str; 6] =
    ["lib", "test", ".dart_tool", ".idea", "build", "README.md"];
const CACHE_ACCESS_MARKER: &str = ".flup-last-used";
const STALE_TEMP_MAX_AGE: Duration = Duration::from_secs(600);
pub const MAX_CACHED_TEMPLATES: usize = 16;

static CACHE_CLOCK_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn apply_templates(
    base: &Path,
    target: &Path,
    stage: &Path,
    platforms: &[String],
    force: bool,
) -> Result<Report> {
    let (old_files, new_files) = std::thread::scope(|s| {
        let old_handle = s.spawn(|| template_files(base, platforms));
        let new_handle = s.spawn(|| template_files(target, platforms));

        let old = old_handle
            .join()
            .map_err(|_| FlupError::Internal("base template reading thread panicked".into()))??;
        let new = new_handle
            .join()
            .map_err(|_| FlupError::Internal("target template reading thread panicked".into()))??;

        Ok::<_, FlupError>((old, new))
    })?;

    let mut paths: BTreeSet<PathBuf> = old_files.keys().cloned().collect();
    paths.extend(new_files.keys().cloned());

    let mut report = Report::default();
    for relative in paths {
        let display = normalize_display_path(&relative);
        let old = old_files.get(&relative);
        let new = new_files.get(&relative);
        let user_path = stage.join(&relative);
        let user = read_optional(&user_path)?;

        // The decision is pure; only approved updates are written to the stage.
        let decision = decide_file(
            old.map(Vec::as_slice),
            new.map(Vec::as_slice),
            user.as_deref(),
        );

        if decision == FileDecision::Conflict {
            if force {
                let content = new.expect("conflict in template has target content");
                write_file(&user_path, content)?;
                report.forced.push(display);
                continue;
            } else {
                record_conflict_preview(
                    &mut report,
                    &display,
                    old.map(Vec::as_slice),
                    new.map(Vec::as_slice),
                    user.as_deref(),
                );
            }
        }

        let is_addition = decision == FileDecision::Add;
        apply_decision(
            decision,
            new.map(Vec::as_slice),
            &user_path,
            display.clone(),
            &mut report,
        )?;

        if let Some(existing_file) = is_addition
            .then(|| detect_coexistence(&relative, stage))
            .flatten()
        {
            let old_display = normalize_display_path(&existing_file);
            report.coexistence_notices.push((display, old_display));
        }
    }
    Ok(report)
}

fn record_conflict_preview(
    report: &mut Report,
    path: &str,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    user: Option<&[u8]>,
) {
    let (Some(old), Some(new), Some(user)) = (old, new, user) else {
        return;
    };
    let (Some(project_changes), Some(flutter_changes)) =
        (diff_preview(old, user), diff_preview(old, new))
    else {
        return;
    };

    report.conflict_previews.insert(
        path.to_string(),
        ConflictPreview {
            project_changes,
            flutter_changes,
        },
    );
}

fn apply_decision(
    decision: FileDecision,
    new_content: Option<&[u8]>,
    user_path: &Path,
    display: String,
    report: &mut Report,
) -> Result<()> {
    match decision {
        FileDecision::Update => {
            let content = new_content.expect("update has target content");
            write_file(user_path, content)?;
            report.updated.push(display);
        }
        FileDecision::Merge(content) => {
            write_file(user_path, &content)?;
            report.merged.push(display);
        }
        FileDecision::Add => {
            let content = new_content.expect("addition has target content");
            write_file(user_path, content)?;
            report.added.push(display);
        }
        FileDecision::RemovedUpstream => report.removed_upstream.push(display),
        FileDecision::Preserve => report.preserved.push(display),
        FileDecision::Conflict => report.conflicts.push(display),
        FileDecision::Ignore => {}
    }
    Ok(())
}

fn detect_coexistence(relative: &Path, stage: &Path) -> Option<PathBuf> {
    let parent = relative.parent()?;
    let stem = relative.file_stem()?.to_str()?;
    let file_name = relative.file_name()?.to_str()?;
    let extension = relative
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    let entries = fs::read_dir(stage.join(parent)).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|ft| ft.is_file()) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == file_name {
                continue;
            }

            let entry_path = Path::new(&*name_str);
            let entry_stem = entry_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let entry_ext = entry_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();

            // Case 1: Template adds compound extension (e.g. build.gradle.kts alongside build.gradle)
            let is_compound_extension = name_str == stem || file_name == entry_stem;

            // Case 2: Same stem, different extension (e.g. MainActivity.kt alongside MainActivity.java)
            let is_format_migration = !stem.is_empty()
                && stem == entry_stem
                && extension != entry_ext
                && !extension.is_empty()
                && !entry_ext.is_empty();

            if is_compound_extension || is_format_migration {
                return Some(parent.join(&*name_str));
            }
        }
    }
    None
}

fn write_file(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            let context = format!("cannot create {}", parent.display());
            FlupError::io(context, error)
        })?;
    }
    fs::write(path, content).map_err(|error| {
        let context = format!("cannot write {}", path.display());
        FlupError::io(context, error)
    })
}

fn template_files(root: &Path, platforms: &[String]) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| is_eligible_entry(entry, platforms))
    {
        let entry = entry.map_err(|error| {
            let message = format!("cannot walk template: {error}");
            FlupError::Template(message)
        })?;
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| {
                let message = format!("template path is outside root: {error}");
                FlupError::Internal(message)
            })?
            .to_path_buf();

        if is_eligible_template_file(&relative, platforms) {
            let content = fs::read(entry.path()).map_err(|error| {
                let context = format!("cannot read {}", entry.path().display());
                FlupError::io(context, error)
            })?;
            files.insert(relative, content);
        }
    }
    Ok(files)
}

fn is_eligible_entry(entry: &walkdir::DirEntry, platforms: &[String]) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    if entry.depth() == 1 {
        if entry.file_type().is_dir() {
            return platforms
                .iter()
                .any(|platform| entry.file_name() == platform.as_str());
        }
        return ROOT_TEMPLATE_FILES
            .iter()
            .any(|root_file| entry.file_name() == *root_file);
    }

    true
}

fn is_eligible_template_file(relative: &Path, platforms: &[String]) -> bool {
    let first = relative
        .components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy());

    let is_platform_file = first
        .as_deref()
        .is_some_and(|name| platforms.iter().any(|platform| platform == name));

    let is_root_template = ROOT_TEMPLATE_FILES
        .iter()
        .any(|root_file| relative == Path::new(root_file));

    is_platform_file || is_root_template
}

fn normalize_display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Instantiate a canonical cached template into `output` using the target project's parameters.
pub fn instantiate_cached_template(
    cached_dir: &Path,
    output: &Path,
    params: &TemplateParameters,
) -> Result<()> {
    touch_cache_entry(cached_dir);

    for entry in WalkDir::new(cached_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_stripped_cache_entry(entry))
    {
        let entry = entry.map_err(|error| {
            let message = format!("cannot walk cached template: {error}");
            FlupError::Template(message)
        })?;
        if entry.depth() == 0 {
            fs::create_dir_all(output).map_err(|error| {
                FlupError::io(format!("cannot create {}", output.display()), error)
            })?;
            continue;
        }
        if entry.file_name() == CACHE_ACCESS_MARKER {
            continue;
        }

        let relative = entry.path().strip_prefix(cached_dir).map_err(|error| {
            FlupError::Internal(format!("cached template path is outside root: {error}"))
        })?;
        let target_relative = params.instantiate_relative_path(relative);
        let destination = output.join(&target_relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination).map_err(|error| {
                FlupError::io(format!("cannot create {}", destination.display()), error)
            })?;
            continue;
        }

        if entry.file_type().is_file() {
            let raw = fs::read(entry.path()).map_err(|error| {
                FlupError::io(format!("cannot read {}", entry.path().display()), error)
            })?;
            let rendered = params.instantiate_content(&target_relative, &raw);
            write_file(&destination, &rendered)?;

            if let Ok(metadata) = entry.metadata() {
                let _ = fs::set_permissions(&destination, metadata.permissions());
            }
        }
    }

    Ok(())
}

/// Store a canonical template into `cached_dir`, stripping non-template directories and pruning old entries.
pub fn store_cached_template(source: &Path, cached_dir: &Path, max_entries: usize) -> Result<()> {
    let cache_root = cached_dir.parent().ok_or_else(|| {
        FlupError::Template(format!(
            "cache path has no parent: {}",
            cached_dir.display()
        ))
    })?;
    fs::create_dir_all(cache_root).map_err(|error| {
        FlupError::io(
            format!("cannot create template cache {}", cache_root.display()),
            error,
        )
    })?;

    if cached_dir.exists() {
        touch_cache_entry(cached_dir);
        prune_template_cache(cache_root, max_entries);
        return Ok(());
    }

    let temp_path = cache_root.join(unique_temp_cache_name("template"));
    let result = (|| {
        copy_filtered_template(source, &temp_path)?;
        touch_cache_entry(&temp_path);
        fs::rename(&temp_path, cached_dir).map_err(|error| {
            FlupError::io(
                format!("cannot store template cache {}", cached_dir.display()),
                error,
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp_path);
    }

    prune_template_cache(cache_root, max_entries);
    result
}

fn copy_filtered_template(source: &Path, destination: &Path) -> Result<()> {
    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_stripped_cache_entry(entry))
    {
        let entry = entry.map_err(|error| {
            FlupError::Template(format!("cannot walk generated template: {error}"))
        })?;
        let relative = entry.path().strip_prefix(source).map_err(|error| {
            FlupError::Internal(format!("template path is outside root: {error}"))
        })?;
        let target = destination.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|error| {
                FlupError::io(format!("cannot create {}", target.display()), error)
            })?;
            continue;
        }

        if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    FlupError::io(format!("cannot create {}", parent.display()), error)
                })?;
            }
            fs::copy(entry.path(), &target).map_err(|error| {
                FlupError::io(format!("cannot copy {}", target.display()), error)
            })?;
        }
    }
    Ok(())
}

fn is_stripped_cache_entry(entry: &walkdir::DirEntry) -> bool {
    entry.depth() == 1
        && NON_TEMPLATE_ENTRIES
            .iter()
            .any(|name| entry.file_name() == *name)
}

/// Record recent access on a cached template directory for LRU eviction.
pub fn touch_cache_entry(cached_dir: &Path) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = CACHE_CLOCK_SEQ.fetch_add(1, Ordering::Relaxed) as u128;
    let stamp = format!("{nanos:032}:{seq:016}");
    let _ = fs::write(cached_dir.join(CACHE_ACCESS_MARKER), stamp);
}

/// Evict least-recently-used template cache directories so the cache never exceeds `max_entries`.
pub fn prune_template_cache(cache_root: &Path, max_entries: usize) {
    let Ok(read_dir) = fs::read_dir(cache_root) else {
        return;
    };

    let now = SystemTime::now();
    let mut entries: Vec<(String, PathBuf)> = Vec::new();

    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with(".tmp-") {
            let is_stale = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_TEMP_MAX_AGE);
            if is_stale {
                let _ = fs::remove_dir_all(&path);
            }
            continue;
        }

        if name_str.starts_with('.') {
            continue;
        }

        let sort_key = read_cache_access_key(&path);
        entries.push((sort_key, path));
    }

    if entries.len() <= max_entries {
        return;
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove_count = entries.len() - max_entries;
    for (_, path) in entries.into_iter().take(remove_count) {
        let _ = fs::remove_dir_all(path);
    }
}

fn read_cache_access_key(cached_dir: &Path) -> String {
    if let Ok(marker) = fs::read_to_string(cached_dir.join(CACHE_ACCESS_MARKER)) {
        let trimmed = marker.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    let nanos = fs::metadata(cached_dir)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:032}:0000000000000000")
}

fn unique_temp_cache_name(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = CACHE_CLOCK_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!(".tmp-{prefix}-{pid}-{now}-{seq}")
}
