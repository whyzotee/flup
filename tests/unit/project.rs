use flup::error::FlupError;
use flup::storage::project_store::{align_environment_sdk, inspect_project, update_metadata};
use flup::upgrade::project::{
    SemVer, align_environment_sdk_content, parse_min_sdk_version, project_name,
    project_organization, project_revision, update_metadata_content,
};
use std::fs;
use std::path::Path;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn metadata_keeps_creation_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".metadata");
    write(
        temp.path(),
        ".metadata",
        "version:\n  revision: old\nmigration:\n  platforms:\n    - platform: android\n      create_revision: old\n      base_revision: old\n",
    );
    update_metadata(&path, "new").unwrap();
    let content = fs::read_to_string(path).unwrap();
    assert!(content.contains("revision: new"));
    assert!(content.contains("create_revision: old"));
    assert!(content.contains("base_revision: new"));
}

#[test]
fn metadata_read_error_retains_io_source() {
    let temp = tempfile::tempdir().unwrap();
    let error = update_metadata(&temp.path().join("missing.metadata"), "new").unwrap_err();
    assert!(
        matches!(error, FlupError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
    );
}

#[test]
fn app_project_reads_base_revision_and_platforms() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "pubspec.yaml", "name: sample\n");
    write(
        temp.path(),
        ".metadata",
        "version:\n  revision: 0123456789abcdef\nproject_type: app\n",
    );
    write(temp.path(), "web/index.html", "<html></html>");
    let (_, project) = inspect_project(temp.path()).unwrap();
    assert_eq!(project.name, "sample");
    assert_eq!(project.revision, "0123456789abcdef");
    assert_eq!(project.platforms, ["web"]);
}

#[test]
fn metadata_revision_update_is_pure() {
    let output = update_metadata_content("revision: old\ncreate_revision: old\n", "new").unwrap();
    assert_eq!(output, "revision: new\ncreate_revision: old\n");
}

#[test]
fn metadata_update_preserves_unrelated_revision() {
    let input = "version:\n  revision: old\nother:\n  revision: keep\nmigration:\n  platforms:\n    - platform: web\n      base_revision: old\n";
    let output = update_metadata_content(input, "new").unwrap();
    assert!(output.contains("version:\n  revision: new"));
    assert!(output.contains("other:\n  revision: keep"));
    assert!(output.contains("base_revision: new"));
}

#[test]
fn different_platform_base_revision_is_rejected_without_filesystem() {
    let error = project_revision(
        "revision: 0123456789abcdef\nproject_type: app\nbase_revision: fedcba9876543210\n",
    )
    .unwrap_err();
    assert!(matches!(error, FlupError::InvalidProject(_)));
}

#[test]
fn organization_conflict_is_rejected_without_filesystem() {
    let sources = vec![
        "namespace = \"com.one.sample\"".into(),
        "PRODUCT_BUNDLE_IDENTIFIER = com.two.sample".into(),
    ];
    let error = project_organization("sample", &sources, true).unwrap_err();
    assert!(matches!(error, FlupError::InvalidProject(_)));
}

#[test]
fn project_name_uses_top_level_yaml_key() {
    let pubspec = "dependencies:\n  sample:\n    name: nested\nname: actual\n";
    assert_eq!(project_name(pubspec).unwrap(), "actual");
}

#[test]
fn inline_path_dependency_is_rejected() {
    let pubspec = "name: sample\ndependencies:\n  local: {path: ../local}\n";
    assert!(matches!(
        project_name(pubspec),
        Err(FlupError::InvalidProject(_))
    ));
}

#[test]
fn workspace_member_is_rejected() {
    assert!(matches!(
        project_name("name: sample\nresolution: workspace\n"),
        Err(FlupError::InvalidProject(_))
    ));
}

#[test]
fn malformed_yaml_retains_parse_source() {
    let error = project_name("name: [broken\n").unwrap_err();
    assert!(matches!(error, FlupError::InvalidYaml { .. }));
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn project_revision_uses_version_section() {
    let metadata =
        "other:\n  revision: bad\nversion:\n  revision: abcdef0123456789\nproject_type: app\n";
    assert_eq!(project_revision(metadata).unwrap(), "abcdef0123456789");
}

#[test]
fn unreadable_identifier_file_is_not_treated_as_missing() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "pubspec.yaml", "name: sample\n");
    write(
        temp.path(),
        ".metadata",
        "version:\n  revision: abcdef0123456789\nproject_type: app\n",
    );
    fs::create_dir_all(temp.path().join("android/app/build.gradle.kts")).unwrap();
    let error = inspect_project(temp.path()).unwrap_err();
    assert!(matches!(error, FlupError::Io { .. }));
}

#[test]
fn project_organization_supports_both_groovy_and_kotlin_dsl() {
    let kotlin_dsl = vec!["applicationId = \"com.mycompany.sample\"".to_string()];
    assert_eq!(
        project_organization("sample", &kotlin_dsl, true).unwrap(),
        "com.mycompany"
    );

    let groovy = vec!["applicationId 'com.mycompany.sample'".to_string()];
    assert_eq!(
        project_organization("sample", &groovy, true).unwrap(),
        "com.mycompany"
    );
}

#[test]
fn inspect_project_detects_ios_language() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "pubspec.yaml", "name: sample\n");
    write(
        temp.path(),
        ".metadata",
        "version:\n  revision: 0123456789abcdef\nproject_type: app\n",
    );
    write(
        temp.path(),
        "ios/Runner.xcodeproj/project.pbxproj",
        "PRODUCT_BUNDLE_IDENTIFIER = com.myorg.sample;\n",
    );
    write(temp.path(), "ios/Runner/AppDelegate.m", "// objc");
    let (_, project) = inspect_project(temp.path()).unwrap();
    assert_eq!(project.ios_language.as_deref(), Some("objc"));

    write(temp.path(), "ios/Runner/AppDelegate.swift", "// swift");
    let (_, project_swift) = inspect_project(temp.path()).unwrap();
    assert_eq!(project_swift.ios_language.as_deref(), Some("swift"));
}

#[test]
fn project_organization_supports_manifest_package_and_casing() {
    let manifest = vec!["<manifest package=\"com.myorg.sample\">".to_string()];
    assert_eq!(
        project_organization("sample", &manifest, true).unwrap(),
        "com.myorg"
    );

    let case_mismatch = vec!["applicationId = \"com.myorg.Sample\"".to_string()];
    assert_eq!(
        project_organization("sample", &case_mismatch, true).unwrap(),
        "com.myorg"
    );

    let custom_suffix = vec!["applicationId = \"com.myorg.custom_name\"".to_string()];
    assert_eq!(
        project_organization("sample", &custom_suffix, true).unwrap(),
        "com.myorg"
    );
}

#[test]
fn inspect_project_detects_kotlin_files_in_java_directory() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "pubspec.yaml", "name: sample\n");
    write(
        temp.path(),
        ".metadata",
        "version:\n  revision: 0123456789abcdef\nproject_type: app\n",
    );
    write(
        temp.path(),
        "android/app/build.gradle",
        "applicationId 'com.example.sample'\n",
    );
    write(
        temp.path(),
        "android/app/src/main/java/com/example/MainActivity.kt",
        "class MainActivity",
    );
    let (_, project) = inspect_project(temp.path()).unwrap();
    assert_eq!(project.android_language.as_deref(), Some("kotlin"));
}

#[test]
fn parse_min_sdk_version_handles_various_formats() {
    assert_eq!(
        parse_min_sdk_version("^3.5.3"),
        Some(SemVer {
            major: 3,
            minor: 5,
            patch: 3
        })
    );
    assert_eq!(
        parse_min_sdk_version("'>=3.5.3 <4.0.0'"),
        Some(SemVer {
            major: 3,
            minor: 5,
            patch: 3
        })
    );
    assert_eq!(
        parse_min_sdk_version("\">=2.19.0 <3.0.0\""),
        Some(SemVer {
            major: 2,
            minor: 19,
            patch: 0
        })
    );
    assert_eq!(
        parse_min_sdk_version("^3.7"),
        Some(SemVer {
            major: 3,
            minor: 7,
            patch: 0
        })
    );
    assert_eq!(parse_min_sdk_version("invalid"), None);
}

#[test]
fn align_environment_sdk_content_bumps_lower_version_and_preserves_comments() {
    let template = "name: sample\nenvironment:\n  sdk: ^3.7.0\n";
    let project = "\
# App header
name: my_app
description: \"Test app\"

environment:
  sdk: ^3.5.3 # Dart floor constraint
  flutter: \">=3.3.0\"

dependencies:
  flutter:
    sdk: flutter
";

    let result = align_environment_sdk_content(template, project).unwrap();
    assert!(result.is_some());
    let updated = result.unwrap();

    assert!(updated.contains("sdk: ^3.7.0 # Dart floor constraint"));
    assert!(updated.contains("# App header"));
    assert!(updated.contains("flutter: \">=3.3.0\""));
    assert!(updated.contains("    sdk: flutter"));
}

#[test]
fn align_environment_sdk_content_preserves_higher_version() {
    let template = "name: sample\nenvironment:\n  sdk: ^3.7.0\n";
    let project = "name: my_app\nenvironment:\n  sdk: ^3.8.0\n";

    let result = align_environment_sdk_content(template, project).unwrap();
    assert!(result.is_none());
}

#[test]
fn align_environment_sdk_content_upgrades_dart2_range() {
    let template = "name: sample\nenvironment:\n  sdk: ^3.7.0\n";
    let project = "name: my_app\nenvironment:\n  sdk: '>=2.19.0 <3.0.0'\n";

    let result = align_environment_sdk_content(template, project).unwrap();
    assert!(result.is_some());
    let updated = result.unwrap();
    assert!(updated.contains("sdk: ^3.7.0"));
}

#[test]
fn align_environment_sdk_updates_stage_file() {
    let temp = tempfile::tempdir().unwrap();
    let template_dir = temp.path().join("template");
    let stage_dir = temp.path().join("stage");

    write(
        &template_dir,
        "pubspec.yaml",
        "environment:\n  sdk: ^3.7.0\n",
    );
    write(
        &stage_dir,
        "pubspec.yaml",
        "name: app\nenvironment:\n  sdk: ^3.5.3\n",
    );

    align_environment_sdk(&template_dir, &stage_dir).unwrap();

    let stage_content = fs::read_to_string(stage_dir.join("pubspec.yaml")).unwrap();
    assert!(stage_content.contains("sdk: ^3.7.0"));
}
