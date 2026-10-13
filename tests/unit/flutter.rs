use flup::error::FlupError;
use flup::flutter::prepare_base_sdk;
use flup::flutter::process::sdk_revision;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn base_sdk_checkout_uses_local_history_without_switching_target() {
    let temp = tempfile::tempdir().unwrap();
    // canonicalize produces a verbatim path on Windows, as project discovery does.
    let workspace = fs::canonicalize(temp.path()).unwrap();
    let target = workspace.join("target-sdk");
    let base = workspace.join("base-sdk");
    fs::create_dir_all(target.join("packages/flutter_tools")).unwrap();
    fs::create_dir_all(target.join("bin")).unwrap();
    fs::write(target.join("packages/flutter_tools/marker"), "tool").unwrap();
    fs::write(target.join("bin/flutter"), "#!/bin/sh\n").unwrap();
    fs::write(target.join("bin/flutter.bat"), "@echo off\n").unwrap();
    fs::write(target.join("marker"), "old").unwrap();

    git(&target, &["init", "--quiet"]);
    git(&target, &["config", "user.name", "Flup Test"]);
    git(&target, &["config", "user.email", "flup@example.test"]);
    git(&target, &["add", "."]);
    git(&target, &["commit", "--quiet", "-m", "base"]);
    let base_revision = git(&target, &["rev-parse", "HEAD"]);

    fs::write(target.join("marker"), "new").unwrap();
    git(&target, &["add", "marker"]);
    git(&target, &["commit", "--quiet", "-m", "target"]);
    let target_revision = git(&target, &["rev-parse", "HEAD"]);

    let resolved = prepare_base_sdk(&workspace, &target, &base_revision, None, &base).unwrap();

    assert_eq!(resolved, fs::canonicalize(&base).unwrap());
    assert_eq!(sdk_revision(&base).unwrap(), base_revision);
    assert_eq!(sdk_revision(&target).unwrap(), target_revision);
    assert_eq!(fs::read_to_string(base.join("marker")).unwrap(), "old");
    assert_eq!(fs::read_to_string(target.join("marker")).unwrap(), "new");
}

#[test]
fn missing_base_revision_does_not_create_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target-sdk");
    let base = temp.path().join("base-sdk");
    fs::create_dir(&target).unwrap();
    git(&target, &["init", "--quiet"]);

    let missing_revision = "0000000000000000000000000000000000000000";
    let error = prepare_base_sdk(temp.path(), &target, missing_revision, None, &base).unwrap_err();

    assert!(matches!(error, FlupError::Sdk(_)));
    assert!(!base.exists());
}

#[test]
fn sdk_version_prefers_version_file() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("version"), "3.27.1\n").unwrap();
    let version = flup::flutter::process::sdk_version(temp.path()).unwrap();
    assert_eq!(version, "3.27.1");
}

#[test]
fn git_migration_branch_is_created_for_git_project() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("my_app");
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "--quiet"]);
    git(&project, &["config", "user.name", "Flup Test"]);
    git(&project, &["config", "user.email", "flup@example.test"]);
    fs::write(project.join("pubspec.yaml"), "name: my_app\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "--quiet", "-m", "initial"]);

    let branch = flup::flutter::process::create_migration_branch(&project, "3.27.1")
        .unwrap()
        .unwrap();
    assert_eq!(branch, "migrate-to-flutter-3.27.1");

    let current_branch = git(&project, &["branch", "--show-current"]);
    assert_eq!(current_branch, "migrate-to-flutter-3.27.1");

    let last_commit = git(&project, &["log", "-1", "--pretty=%B"]);
    assert_eq!(last_commit, "initial");
}

#[test]
fn git_migration_branch_is_skipped_when_not_a_git_repository() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("non_git_app");
    fs::create_dir_all(&project).unwrap();

    let branch = flup::flutter::process::create_migration_branch(&project, "3.27.1").unwrap();
    assert_eq!(branch, None);
}
