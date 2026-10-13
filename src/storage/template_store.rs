use crate::error::{FlupError, Result};
use crate::storage::fs_util::read_optional;
use crate::upgrade::template::{ConflictPreview, FileDecision, Report, decide_file};
use crate::upgrade::text_merge::diff_preview;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const ROOT_TEMPLATE_FILES: [&str; 2] = ["analysis_options.yaml", ".gitignore"];

pub fn apply_templates(
    base: &Path,
    target: &Path,
    stage: &Path,
    platforms: &[String],
    force: bool,
) -> Result<Report> {
    let old_files = template_files(base, platforms)?;
    let new_files = template_files(target, platforms)?;
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
    for entry in WalkDir::new(root).follow_links(false) {
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
