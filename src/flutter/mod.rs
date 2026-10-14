pub mod process;

use self::process::sdk_revision;
use crate::error::{FlupError, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_sdk(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return required_sdk_root(path);
    }

    if let Some(value) = env::var_os("PATH") {
        for directory in env::split_paths(&value) {
            if let Some(root) = sdk_root(&directory) {
                return Ok(root);
            }
        }
    }

    for path in sdk_candidates(None) {
        if let Some(root) = sdk_root(&path) {
            return Ok(root);
        }
    }

    Err(FlupError::Sdk(
        "Flutter SDK not found; pass --flutter <SDK path>".into(),
    ))
}

pub fn prepare_base_sdk(
    project: &Path,
    target_sdk: &Path,
    revision: &str,
    explicit: Option<&Path>,
    workspace: &Path,
) -> Result<PathBuf> {
    if let Some(root) = find_installed_base_sdk(project, revision, explicit)? {
        return Ok(root);
    }

    process::checkout_sdk_revision(target_sdk, revision, workspace)?;
    required_sdk_root(workspace)
}

/// Resolve an installed base SDK or verify that `revision` exists in `target_sdk` local Git history,
/// deferring the working-tree checkout into `workspace` until a template cache miss occurs.
pub fn prepare_lazy_base_sdk(
    project: &Path,
    target_sdk: &Path,
    revision: &str,
    explicit: Option<&Path>,
    workspace: &Path,
) -> Result<PathBuf> {
    if let Some(root) = find_installed_base_sdk(project, revision, explicit)? {
        return Ok(root);
    }

    process::verify_sdk_revision_in_history(target_sdk, revision)?;
    Ok(workspace.to_path_buf())
}

fn find_installed_base_sdk(
    project: &Path,
    revision: &str,
    explicit: Option<&Path>,
) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        let root = required_sdk_root(path)?;
        let actual_revision = sdk_revision(&root)?;

        if actual_revision != revision {
            let message = format!("--base-flutter revision does not match .metadata ({revision})");
            return Err(FlupError::Sdk(message));
        }

        return Ok(Some(root));
    }

    for path in sdk_candidates(Some(project)) {
        let Some(root) = sdk_root(&path) else {
            continue;
        };

        let revision_matches = sdk_revision(&root).ok().as_deref() == Some(revision);

        if revision_matches {
            return Ok(Some(root));
        }
    }

    Ok(None)
}

fn sdk_candidates(project: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(value) = env::var_os("FLUTTER_ROOT") {
        paths.push(PathBuf::from(value));
    }

    if let Some(project) = project {
        paths.push(project.join(".fvm/flutter_sdk"));
    }

    if let Some(value) = env::var_os("PATH") {
        for dir in env::split_paths(&value) {
            paths.push(dir.join("flutter"));
            paths.push(dir);
        }
    }

    if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        let home = PathBuf::from(home);
        paths.push(home.join("development/flutter"));
        paths.push(home.join("fvm/versions"));
        paths.push(home.join(".fvm/versions"));
    }

    paths.push(PathBuf::from("C:/dev/flutter"));

    expand_version_directories(paths)
}

fn expand_version_directories(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    // Version managers store several SDKs below one directory.
    let mut expanded = Vec::new();
    for path in paths {
        let is_versions_directory =
            path.is_dir() && path.file_name().is_some_and(|name| name == "versions");

        if is_versions_directory {
            if let Ok(entries) = fs::read_dir(&path) {
                expanded.extend(entries.flatten().map(|e| e.path()));
            }
        } else {
            expanded.push(path);
        }
    }

    expanded
}

fn sdk_root(path: &Path) -> Option<PathBuf> {
    check_sdk_directory(path).or_else(|| check_sdk_directory(path.parent()?))
}

fn check_sdk_directory(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() {
        return None;
    }

    let root = fs::canonicalize(path).ok()?;
    let has_flutter_tools = root.join("packages/flutter_tools").is_dir();
    let has_flutter_binary = flutter_binary(&root).exists();

    if has_flutter_tools && has_flutter_binary {
        Some(root)
    } else {
        None
    }
}

fn required_sdk_root(path: &Path) -> Result<PathBuf> {
    sdk_root(path).ok_or_else(|| {
        let message = format!("not a Flutter SDK: {}", path.display());
        FlupError::Sdk(message)
    })
}

pub fn flutter_binary(root: &Path) -> PathBuf {
    if cfg!(windows) {
        process::command_path(&root.join("bin/flutter.bat"))
    } else {
        root.join("bin/flutter")
    }
}
