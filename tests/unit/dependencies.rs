use flup::error::FlupError;
use flup::upgrade::dependencies::changed_dependency_files;
use std::fs;

#[test]
fn dependency_comparison_propagates_read_errors() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let stage = temp.path().join("stage");
    fs::create_dir_all(project.join("pubspec.yaml")).unwrap();
    fs::create_dir_all(&stage).unwrap();
    let error = changed_dependency_files(&project, &stage).unwrap_err();
    assert!(matches!(error, FlupError::Io { .. }));
}
