use crate::error::{FlupError, Result};
use crate::flutter::flutter_binary;
use crate::storage::template_store::{
    MAX_CACHED_TEMPLATES, instantiate_cached_template, store_cached_template,
};
use crate::upgrade::project::ProjectInfo;
use crate::upgrade::template::{TemplateParameters, canonical_project_info};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub fn sdk_revision(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(command_path(root))
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| {
            let context = format!("cannot inspect Flutter SDK {}", root.display());
            FlupError::io(context, error)
        })?;
    if !output.status.success() {
        let message = format!("Flutter SDK {} has no Git revision", root.display());
        return Err(FlupError::Sdk(message));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn sdk_version(root: &Path) -> Result<String> {
    let version_file = root.join("version");
    if let Ok(content) = fs::read_to_string(&version_file) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(command_path(root))
        .args(["describe", "--tags", "--always"])
        .output();

    if let Some(output) = output.ok().filter(|out| out.status.success()) {
        let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !tag.is_empty() {
            return Ok(tag);
        }
    }

    sdk_revision(root)
}

pub fn is_git_repository(path: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(command_path(path))
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == b"true")
}

pub fn create_migration_branch(project: &Path, target_version: &str) -> Result<Option<String>> {
    if !is_git_repository(project) {
        return Ok(None);
    }

    let branch_name = format!("migrate-to-flutter-{target_version}");
    let branch_exists = Command::new("git")
        .arg("-C")
        .arg(command_path(project))
        .args([
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch_name}"),
        ])
        .output()
        .is_ok_and(|out| out.status.success());

    let checkout_args = if branch_exists {
        vec!["checkout", branch_name.as_str()]
    } else {
        vec!["checkout", "-b", branch_name.as_str()]
    };

    let output = Command::new("git")
        .arg("-C")
        .arg(command_path(project))
        .args(&checkout_args)
        .output()
        .map_err(|e| FlupError::io(format!("cannot switch to branch {branch_name}"), e))?;

    if !output.status.success() {
        let message = format!(
            "failed to switch to git branch {branch_name}:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Err(FlupError::Commit(message));
    }

    Ok(Some(branch_name))
}

/// Verify that `revision` exists as a commit in `target`'s local Git object store.
pub fn verify_sdk_revision_in_history(target: &Path, revision: &str) -> Result<()> {
    let object = Command::new("git")
        .arg("-C")
        .arg(command_path(target))
        .args(["cat-file", "-t", revision])
        .output()
        .map_err(|error| FlupError::io("cannot inspect Flutter SDK Git history", error))?;

    let is_local_commit = object.status.success() && object.stdout.trim_ascii() == b"commit";
    if !is_local_commit {
        let message = format!(
            "Flutter SDK revision {revision} is not available in local Git history; install that SDK and pass --base-flutter <SDK path>"
        );
        return Err(FlupError::Sdk(message));
    }

    Ok(())
}

/// Recreate an older SDK from the target SDK's local Git objects.
pub fn checkout_sdk_revision(target: &Path, revision: &str, destination: &Path) -> Result<()> {
    verify_sdk_revision_in_history(target, revision)?;

    let clone = Command::new("git")
        .current_dir(target)
        .args(["clone", "--shared", "--no-checkout", "--quiet"])
        // A Windows drive path can be parsed as an SSH-style remote by Git.
        .arg(".")
        .arg(command_path(destination))
        .output()
        .map_err(|error| {
            FlupError::io("cannot clone Flutter SDK into temporary workspace", error)
        })?;

    if !clone.status.success() {
        let message = format!(
            "cannot clone Flutter SDK into temporary workspace: {}",
            String::from_utf8_lossy(&clone.stderr)
        );
        return Err(FlupError::Sdk(message));
    }

    let checkout = Command::new("git")
        .arg("-C")
        .arg(command_path(destination))
        .args(["checkout", "--detach", "--quiet", revision])
        .output()
        .map_err(|error| FlupError::io("cannot check out base Flutter SDK revision", error))?;

    if !checkout.status.success() {
        let message = format!(
            "cannot check out Flutter SDK revision {revision}: {}",
            String::from_utf8_lossy(&checkout.stderr)
        );
        return Err(FlupError::Sdk(message));
    }

    let actual_revision = sdk_revision(destination)?;
    if actual_revision != revision {
        let message = format!(
            "temporary Flutter SDK revision {actual_revision} does not match .metadata ({revision})"
        );
        return Err(FlupError::Sdk(message));
    }

    Ok(())
}

/// External Windows tools may reject Rust's canonical `\\?\` path prefix.
pub(crate) fn command_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
        let verbatim_unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();

        if let Some(rest) = wide.strip_prefix(&verbatim_unc[..]) {
            let mut conventional = r"\\".encode_utf16().collect::<Vec<_>>();
            conventional.extend_from_slice(rest);
            return PathBuf::from(OsString::from_wide(&conventional));
        }

        if let Some(rest) = wide.strip_prefix(&verbatim[..]) {
            return PathBuf::from(OsString::from_wide(rest));
        }
    }

    path.to_path_buf()
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3u64);
    }
    hash
}

/// Compute a reusable cache key from the SDK revision, platforms, and native language options.
/// Project name and organization are excluded because cached templates use canonical placeholders.
pub fn template_cache_key(revision: &str, info: &ProjectInfo) -> String {
    let mut sorted_platforms = info.platforms.clone();
    sorted_platforms.sort();

    let key_string = format!(
        "v2:{}:{}:{}:{}",
        revision,
        sorted_platforms.join(","),
        info.android_language.as_deref().unwrap_or(""),
        info.ios_language.as_deref().unwrap_or("")
    );
    let hash = fnv1a_64(key_string.as_bytes());
    let sanitized_rev: String = revision
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{sanitized_rev}-{hash:016x}")
}

fn template_cache_root() -> Option<PathBuf> {
    if std::env::var_os("FLUP_NO_CACHE").is_some() {
        return None;
    }

    if let Some(dir) = std::env::var_os("FLUP_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }

    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("LOCALAPPDATA") {
            return Some(PathBuf::from(appdata).join("flup").join("templates"));
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(cache_home) = std::env::var_os("XDG_CACHE_HOME") {
            return Some(PathBuf::from(cache_home).join("flup").join("templates"));
        }
    }

    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".flup").join("templates"))
}

/// Generate the base template for `info.revision`, checking out `base_sdk` from `target_sdk`
/// only when the template is not already cached and `base_sdk` does not exist yet.
pub fn generate_base_template(
    base_sdk: &Path,
    target_sdk: &Path,
    output: &Path,
    info: &ProjectInfo,
) -> Result<()> {
    generate_base_template_with_progress(base_sdk, target_sdk, output, info, &mut |_| {})
}

pub fn generate_base_template_with_progress(
    base_sdk: &Path,
    target_sdk: &Path,
    output: &Path,
    info: &ProjectInfo,
    on_progress: &mut dyn FnMut(String),
) -> Result<()> {
    if try_restore_cached_template(&info.revision, info, output, base_sdk, on_progress) {
        return Ok(());
    }

    if !base_sdk.exists() {
        on_progress(format!("Checking out base SDK {}", info.revision));
        let started = Instant::now();
        let result = checkout_sdk_revision(target_sdk, &info.revision, base_sdk);
        on_progress(format!(
            "Base SDK checkout {} ({:.1}s)",
            if result.is_ok() {
                "completed"
            } else {
                "failed"
            },
            started.elapsed().as_secs_f64()
        ));
        result?;
    }

    // The cache has already been checked before the potentially expensive checkout.
    generate_template_for_revision(base_sdk, Some(&info.revision), output, info, on_progress)
}

pub fn generate_template(sdk: &Path, output: &Path, info: &ProjectInfo) -> Result<()> {
    generate_template_with_progress(sdk, output, info, &mut |_| {})
}

pub fn generate_template_with_progress(
    sdk: &Path,
    output: &Path,
    info: &ProjectInfo,
    on_progress: &mut dyn FnMut(String),
) -> Result<()> {
    let revision = sdk_revision(sdk).ok();
    if let Some(rev) = revision.as_deref()
        && try_restore_cached_template(rev, info, output, sdk, on_progress)
    {
        return Ok(());
    }
    generate_template_for_revision(sdk, revision.as_deref(), output, info, on_progress)
}

fn generate_template_for_revision(
    sdk: &Path,
    revision: Option<&str>,
    output: &Path,
    info: &ProjectInfo,
    on_progress: &mut dyn FnMut(String),
) -> Result<()> {
    if let (Some(rev), Some(cache_root)) = (revision, template_cache_root()) {
        let canonical_info = canonical_project_info(info);
        let canonical_dir = canonical_staging_path(output);
        let _ = fs::remove_dir_all(&canonical_dir);

        timed_flutter_create(sdk, &canonical_dir, &canonical_info, on_progress)?;
        verify_template_output(&canonical_dir, &info.platforms, sdk)?;

        let cached_dir = cache_root.join(template_cache_key(rev, info));
        let started = Instant::now();
        match store_cached_template(&canonical_dir, &cached_dir, MAX_CACHED_TEMPLATES) {
            Ok(()) => on_progress(format!(
                "Template cache stored at {} ({:.1}s)",
                cached_dir.display(),
                started.elapsed().as_secs_f64()
            )),
            Err(error) => on_progress(format!(
                "Warning: cannot save template cache at {}: {error}; continuing without caching",
                cached_dir.display()
            )),
        }

        let params = TemplateParameters::from_project(info);
        let instantiate_res = instantiate_cached_template(&canonical_dir, output, &params);
        let _ = fs::remove_dir_all(&canonical_dir);
        instantiate_res?;

        verify_template_output(output, &info.platforms, sdk)?;
        return Ok(());
    }

    timed_flutter_create(sdk, output, info, on_progress)?;
    verify_template_output(output, &info.platforms, sdk)
}

fn try_restore_cached_template(
    revision: &str,
    info: &ProjectInfo,
    output: &Path,
    sdk_for_error: &Path,
    on_progress: &mut dyn FnMut(String),
) -> bool {
    let started = Instant::now();
    let Some(cache_root) = template_cache_root() else {
        on_progress("Template cache disabled or unavailable".into());
        return false;
    };
    let key = template_cache_key(revision, info);
    let cached_dir = cache_root.join(key);
    if verify_template_output(&cached_dir, &info.platforms, sdk_for_error).is_err() {
        on_progress(format!(
            "Template cache miss: {} ({:.1}s)",
            cached_dir.display(),
            started.elapsed().as_secs_f64()
        ));
        return false;
    }

    let params = TemplateParameters::from_project(info);
    if instantiate_cached_template(&cached_dir, output, &params).is_ok()
        && verify_template_output(output, &info.platforms, sdk_for_error).is_ok()
    {
        on_progress(format!(
            "Template cache hit: {} ({:.1}s)",
            cached_dir.display(),
            started.elapsed().as_secs_f64()
        ));
        return true;
    }

    let _ = fs::remove_dir_all(output);
    on_progress(format!(
        "Template cache could not be restored at {}; regenerating ({:.1}s)",
        cached_dir.display(),
        started.elapsed().as_secs_f64()
    ));
    false
}

fn canonical_staging_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("template");
    output.with_file_name(format!("{name}-canonical"))
}

fn timed_flutter_create(
    sdk: &Path,
    output: &Path,
    info: &ProjectInfo,
    on_progress: &mut dyn FnMut(String),
) -> Result<()> {
    on_progress("Running flutter create --no-pub".into());
    let started = Instant::now();
    let result = run_flutter_create(sdk, output, info);
    on_progress(format!(
        "flutter create {} ({:.1}s)",
        if result.is_ok() {
            "completed"
        } else {
            "failed"
        },
        started.elapsed().as_secs_f64()
    ));
    result
}

fn run_flutter_create(sdk: &Path, output: &Path, info: &ProjectInfo) -> Result<()> {
    let args = build_template_args(info, output);
    let result = Command::new(flutter_binary(sdk))
        .env("FLUTTER_SUPPRESS_ANALYTICS", "true")
        .args(args)
        .output()
        .map_err(|error| {
            let context = format!("failed to launch Flutter SDK {}", sdk.display());
            FlupError::io(context, error)
        })?;
    if !result.status.success() {
        let message = format!(
            "flutter create failed with SDK {}:\n{}",
            sdk.display(),
            String::from_utf8_lossy(&result.stderr)
        );
        return Err(FlupError::Template(message));
    }
    Ok(())
}

fn build_template_args(info: &ProjectInfo, output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "create".into(),
        "--no-pub".into(),
        "--template=app".into(),
        "--project-name".into(),
        info.name.clone().into(),
        "--org".into(),
        info.organization.clone().into(),
        format!("--platforms={}", info.platforms.join(",")).into(),
    ];

    if let Some(language) = &info.android_language {
        args.push(format!("--android-language={language}").into());
    }

    if let Some(language) = &info.ios_language {
        args.push(format!("--ios-language={language}").into());
    }

    args.push(command_path(output).into_os_string());
    args
}

fn verify_template_output(output: &Path, platforms: &[String], sdk: &Path) -> Result<()> {
    // A successful process exit alone does not prove the requested app was generated.
    let has_metadata = output.join(".metadata").is_file();
    let has_missing_platform = platforms
        .iter()
        .any(|platform| !output.join(platform).is_dir());

    if !has_metadata || has_missing_platform {
        let message = format!(
            "Flutter SDK {} did not generate the requested app template and platforms",
            sdk.display()
        );
        return Err(FlupError::Template(message));
    }
    Ok(())
}

pub fn run_flutter(sdk: &Path, cwd: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new(flutter_binary(sdk))
        .env("FLUTTER_SUPPRESS_ANALYTICS", "true")
        .current_dir(command_path(cwd))
        .args(args)
        .output()
        .map_err(|e| FlupError::io("failed to launch flutter", e))?;

    if !result.status.success() {
        let message = format!(
            "flutter {} failed; original project was not changed:\n{}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return Err(FlupError::Dependency(message));
    }

    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}
