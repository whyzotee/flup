use flup::error::FlupError;
use flup::storage::commit::commit_files;
use flup::storage::transaction::Transaction;
use std::fs;
use std::path::{Path, PathBuf};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn transaction_stages_full_copy_and_cleans_up() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    write(&project, "lib/main.dart", "user code");
    write(&project, ".git/config", "git config");

    let transaction = Transaction::begin(&project).unwrap();
    let workspace = transaction.workspace_path("backup");

    assert_eq!(
        fs::read_to_string(workspace.join("lib/main.dart")).unwrap(),
        "user code"
    );
    assert_eq!(
        fs::read_to_string(transaction.stage_path().join(".git/config")).unwrap(),
        "git config"
    );
    let root = workspace.parent().unwrap().to_path_buf();
    drop(transaction);

    assert!(!root.exists());
}

#[test]
fn commit_changes_only_selected_files() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let backup = temp.path().join("backup");
    let stage = temp.path().join("stage");
    for root in [&project, &backup, &stage] {
        write(root, "android/a.txt", "old");
        write(root, "lib/main.dart", "user code");
        write(root, ".metadata", "old revision");
        write(root, "pubspec.yaml", "old constraints");
    }

    write(&stage, "android/a.txt", "new");
    write(&stage, "lib/main.dart", "stage side effect");
    write(&stage, ".metadata", "new revision");
    write(&stage, "pubspec.yaml", "new constraints");
    write(&stage, "pubspec.lock", "new lock");

    let changed = ["android/a.txt", ".metadata", "pubspec.yaml", "pubspec.lock"].map(PathBuf::from);
    commit_files(&project, &stage, &backup, &changed).unwrap();

    assert_eq!(
        fs::read_to_string(project.join("android/a.txt")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(project.join("lib/main.dart")).unwrap(),
        "user code"
    );
    assert_eq!(
        fs::read_to_string(project.join("pubspec.lock")).unwrap(),
        "new lock"
    );
}

#[test]
fn commit_rejects_project_changed_after_staging() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let backup = temp.path().join("backup");
    let stage = temp.path().join("stage");
    for root in [&project, &backup, &stage] {
        write(root, ".metadata", "old");
        write(root, "pubspec.yaml", "old");
    }

    write(&project, "pubspec.yaml", "user changed during upgrade");
    write(&stage, ".metadata", "new");
    write(&stage, "pubspec.yaml", "new");
    let changed = [PathBuf::from(".metadata"), PathBuf::from("pubspec.yaml")];
    let error = commit_files(&project, &stage, &backup, &changed).unwrap_err();

    assert!(
        matches!(error, FlupError::Commit(message) if message.contains("project changed during upgrade"))
    );
    assert_eq!(
        fs::read_to_string(project.join(".metadata")).unwrap(),
        "old"
    );
}

#[test]
fn commit_rejects_paths_outside_project() {
    let temp = tempfile::tempdir().unwrap();
    let changed = [PathBuf::from("../escape.txt")];
    let error = commit_files(temp.path(), temp.path(), temp.path(), &changed).unwrap_err();

    assert!(
        matches!(error, FlupError::Commit(message) if message.contains("invalid relative path"))
    );
}

#[cfg(any(unix, windows))]
#[test]
fn commit_rejects_symlink_replacing_original_file() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let backup = temp.path().join("backup");
    let stage = temp.path().join("stage");
    for root in [&project, &backup, &stage] {
        write(root, "lib/a.txt", "old");
    }

    write(temp.path(), "outside.txt", "old");
    fs::write(stage.join("lib/a.txt"), "new").unwrap();
    fs::remove_file(project.join("lib/a.txt")).unwrap();
    let link = project.join("lib/a.txt");
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(temp.path().join("outside.txt"), &link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_file(temp.path().join("outside.txt"), &link);
    if let Err(error) = result {
        if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("cannot create test symlink: {error}");
    }

    let error = commit_files(&project, &stage, &backup, &[PathBuf::from("lib/a.txt")]).unwrap_err();

    assert!(matches!(error, FlupError::Commit(message) if message.contains("symbolic link")));
    assert_eq!(
        fs::read_to_string(temp.path().join("outside.txt")).unwrap(),
        "old"
    );
}

#[test]
fn commit_deletes_files_removed_from_stage() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let backup = temp.path().join("backup");
    let stage = temp.path().join("stage");
    for root in [&project, &backup, &stage] {
        write(root, "android/build.gradle", "old groovy");
        write(root, ".metadata", "old");
    }

    fs::remove_file(stage.join("android/build.gradle")).unwrap();
    write(&stage, "android/build.gradle.kts", "new kts");

    let changed = [
        PathBuf::from("android/build.gradle"),
        PathBuf::from("android/build.gradle.kts"),
    ];
    commit_files(&project, &stage, &backup, &changed).unwrap();

    assert!(!project.join("android/build.gradle").exists());
    assert_eq!(
        fs::read_to_string(project.join("android/build.gradle.kts")).unwrap(),
        "new kts"
    );
}

#[test]
fn commit_restores_deleted_file_on_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let backup = temp.path().join("backup");
    let stage = temp.path().join("stage");
    for root in [&project, &backup, &stage] {
        write(root, "android/build.gradle", "original groovy");
        write(root, "lib/broken.txt", "initial");
    }

    fs::remove_file(stage.join("android/build.gradle")).unwrap();
    write(&stage, "lib/broken.txt", "modified in stage");

    // Modify lib/broken.txt in project after staging to cause commit failure
    write(&project, "lib/broken.txt", "user changed during staging");

    let changed = [
        PathBuf::from("android/build.gradle"),
        PathBuf::from("lib/broken.txt"),
    ];
    let result = commit_files(&project, &stage, &backup, &changed);
    assert!(result.is_err());

    // Both files should be restored to their initial state
    assert_eq!(
        fs::read_to_string(project.join("android/build.gradle")).unwrap(),
        "original groovy"
    );
}
