use crate::error::{FlupError, Result};
use crate::flutter::flutter_binary;
use crate::upgrade::project::ProjectInfo;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

pub fn commit_git_changes(
    project: &Path,
    changed_paths: &[PathBuf],
    target_version: &str,
) -> Result<()> {
    if !is_git_repository(project) {
        return Ok(());
    }

    let mut add_command = Command::new("git");
    add_command.arg("-C").arg(command_path(project)).arg("add");
    let changed_set: std::collections::BTreeSet<&PathBuf> = changed_paths.iter().collect();
    for path in changed_set {
        add_command.arg(path);
    }
    let add_result = add_command
        .output()
        .map_err(|e| FlupError::io("cannot stage files for git commit", e))?;

    if !add_result.status.success() {
        let message = format!(
            "git add failed:\n{}",
            String::from_utf8_lossy(&add_result.stderr).trim()
        );
        return Err(FlupError::Commit(message));
    }

    let commit_message = format!("Upgrade Flutter to {target_version}");
    let commit_result = Command::new("git")
        .arg("-C")
        .arg(command_path(project))
        .args(["commit", "-m", &commit_message])
        .output()
        .map_err(|e| FlupError::io("cannot create git commit", e))?;

    if !commit_result.status.success() {
        let message = format!(
            "git commit failed:\n{}",
            String::from_utf8_lossy(&commit_result.stderr).trim()
        );
        return Err(FlupError::Commit(message));
    }

    Ok(())
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

pub fn generate_template(sdk: &Path, output: &Path, info: &ProjectInfo) -> Result<()> {
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

    verify_template_output(output, &info.platforms, sdk)
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
