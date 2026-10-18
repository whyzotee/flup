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
fn transaction_stages_project_files_and_cleans_up() {
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
fn transaction_excludes_generated_directories_and_preserves_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let excluded = [
        "build",
        ".dart_tool",
        "android/.gradle",
        "android/build",
        "android/app/build",
        "ios/Flutter/ephemeral",
        "macos/Flutter/ephemeral",
        "windows/flutter/ephemeral",
        "linux/flutter/ephemeral",
    ];
    let retained = [
        ".git/config",
        "lib/main.dart",
        "assets/logo.png",
        "pubspec.yaml",
        "pubspec.lock",
        ".metadata",
        "android/gradle.properties",
        "android/app/build.gradle",
        "android/key.properties",
        "android/app/release.keystore",
        "ios/Pods/Manifest.lock",
        "macos/Pods/Manifest.lock",
        "ios/Flutter/Debug.xcconfig",
        "windows/flutter/CMakeLists.txt",
        "lib/build/user.dart",
        "lib/.dart_tool/user.dart",
        "assets/android/build/user.txt",
        "android/app/src/build/user.txt",
        "ios/Flutter/ephemeral_notes/user.txt",
    ];
    for directory in excluded {
        write(
            &project,
            &format!("{directory}/nested/generated.txt"),
            directory,
        );
    }
    for relative in retained {
        write(&project, relative, relative);
    }

    let transaction = Transaction::begin(&project).unwrap();
    let workspace_root = transaction
        .workspace_path("backup")
        .parent()
        .unwrap()
        .to_path_buf();
    for snapshot in [
        transaction.workspace_path("backup"),
        transaction.stage_path(),
    ] {
        for directory in excluded {
            assert!(!snapshot.join(directory).exists(), "copied {directory}");
        }
        for relative in retained {
            assert_eq!(
                fs::read_to_string(snapshot.join(relative)).unwrap(),
                relative
            );
        }
    }
    drop(transaction);

    assert!(!workspace_root.exists());
    for directory in excluded {
        assert_eq!(
            fs::read_to_string(project.join(directory).join("nested/generated.txt")).unwrap(),
            directory
        );
    }
    for relative in retained {
        assert_eq!(
            fs::read_to_string(project.join(relative)).unwrap(),
            relative
        );
    }
}

#[test]
fn transaction_preserves_regular_files_at_generated_directory_paths() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let paths = [
        "build",
        ".dart_tool",
        "android/.gradle",
        "android/build",
        "android/app/build",
        "ios/Flutter/ephemeral",
        "macos/Flutter/ephemeral",
        "windows/flutter/ephemeral",
        "linux/flutter/ephemeral",
    ];
    for relative in paths {
        write(&project, relative, "user file");
    }

    let transaction = Transaction::begin(&project).unwrap();
    for snapshot in [
        transaction.workspace_path("backup"),
        transaction.stage_path(),
    ] {
        for relative in paths {
            assert_eq!(
                fs::read_to_string(snapshot.join(relative)).unwrap(),
                "user file"
            );
        }
    }
}

#[cfg(any(unix, windows))]
#[test]
fn transaction_skips_generated_directory_symlinks_without_touching_targets() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let generated = temp.path().join("external-build");
    write(&project, "lib/main.dart", "user code");
    write(&generated, "artifact.txt", "generated artifact");
    let link = project.join("build");
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&generated, &link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(&generated, &link);
    if let Err(error) = result {
        if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("cannot create test symlink: {error}");
    }

    let transaction = Transaction::begin(&project).unwrap();
    for snapshot in [
        transaction.workspace_path("backup"),
        transaction.stage_path(),
    ] {
        assert!(fs::symlink_metadata(snapshot.join("build")).is_err());
        assert_eq!(
            fs::read_to_string(snapshot.join("lib/main.dart")).unwrap(),
            "user code"
        );
    }
    drop(transaction);

    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(generated.join("artifact.txt")).unwrap(),
        "generated artifact"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn failed_staging_cleans_workspace_without_changing_original_or_generated_files() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    write(&project, "lib/main.dart", "user code");
    write(&project, "build/artifact.txt", "generated artifact");
    write(temp.path(), "outside.txt", "outside content");
    let link = project.join("lib/unsupported-link");
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

    assert!(matches!(
        Transaction::begin(&project),
        Err(FlupError::InvalidProject(_))
    ));

    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    assert_eq!(
        fs::read_to_string(project.join("lib/main.dart")).unwrap(),
        "user code"
    );
    assert_eq!(
        fs::read_to_string(project.join("build/artifact.txt")).unwrap(),
        "generated artifact"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("outside.txt")).unwrap(),
        "outside content"
    );
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
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
