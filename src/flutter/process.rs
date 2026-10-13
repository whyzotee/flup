use crate::error::{FlupError, Result};
use crate::flutter::flutter_binary;
use crate::storage::tree_copy::copy_tree;
use crate::upgrade::project::ProjectInfo;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Recreate an older SDK from the target SDK's local Git objects.
pub fn checkout_sdk_revision(target: &Path, revision: &str, destination: &Path) -> Result<()> {
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

pub fn template_cache_key(revision: &str, info: &ProjectInfo) -> String {
    let mut sorted_platforms = info.platforms.clone();
    sorted_platforms.sort();

    let key_string = format!(
        "{}:{}:{}:{}:{}:{}",
        revision,
        info.name,
        info.organization,
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

fn unique_temp_cache_name(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!(".tmp-{prefix}-{pid}-{now}")
}

fn store_template_cache(source: &Path, cached_dir: &Path) {
    if cached_dir.exists() {
        return;
    }
    let Some(parent) = cached_dir.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }

    let temp_name = unique_temp_cache_name("template");
    let temp_path = parent.join(temp_name);

    if copy_tree(source, &temp_path).is_ok() {
        if fs::rename(&temp_path, cached_dir).is_err() {
            let _ = fs::remove_dir_all(&temp_path);
        }
    } else {
        let _ = fs::remove_dir_all(&temp_path);
    }
}

pub fn generate_template(sdk: &Path, output: &Path, info: &ProjectInfo) -> Result<()> {
    // 1. Try local template cache
    if let (Ok(revision), Some(cache_root)) = (sdk_revision(sdk), template_cache_root()) {
        let key = template_cache_key(&revision, info);
        let cached_dir = cache_root.join(key);
        if verify_template_output(&cached_dir, &info.platforms, sdk).is_ok() {
            if copy_tree(&cached_dir, output).is_ok()
                && verify_template_output(output, &info.platforms, sdk).is_ok()
            {
                return Ok(());
            }
            let _ = fs::remove_dir_all(output);
        }
    }

    // 2. Cache miss: run flutter create
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

    verify_template_output(output, &info.platforms, sdk)?;

    // 3. Cache the verified template for subsequent runs
    if let (Ok(revision), Some(cache_root)) = (sdk_revision(sdk), template_cache_root()) {
        let key = template_cache_key(&revision, info);
        let cached_dir = cache_root.join(key);
        store_template_cache(output, &cached_dir);
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
