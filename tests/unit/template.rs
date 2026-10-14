use flup::storage::template_store::{
    apply_templates, instantiate_cached_template, prune_template_cache, store_cached_template,
    touch_cache_entry,
};
use flup::upgrade::project::ProjectInfo;
use flup::upgrade::template::{FileDecision, TemplateParameters, decide_file};
use flup::upgrade::text_merge::merge_text;
use std::fs;
use std::path::Path;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn template_diff_updates_only_unchanged_user_files() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    write(&base, "android/a.txt", "old");
    write(&base, "android/b.txt", "old");
    write(&base, "android/c.txt", "old");
    write(&target, "android/a.txt", "new");
    write(&target, "android/b.txt", "new");
    write(&target, "android/c.txt", "old");
    write(&target, "android/d.txt", "added");

    write(&stage, "android/a.txt", "old");
    write(&stage, "android/b.txt", "custom");

    let report = apply_templates(&base, &target, &stage, &["android".into()], false).unwrap();
    assert_eq!(report.updated, ["android/a.txt"]);
    assert_eq!(report.added, ["android/d.txt"]);
    assert_eq!(report.conflicts, ["android/b.txt"]);
    assert_eq!(
        fs::read_to_string(stage.join("android/a.txt")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(stage.join("android/b.txt")).unwrap(),
        "custom"
    );
    assert!(!stage.join("android/c.txt").exists());
}

#[test]
fn conflict_report_shows_both_sides_relative_to_old_template() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    write(&base, "android/config.txt", "name=old\n");
    write(&target, "android/config.txt", "name=flutter\n");
    write(&stage, "android/config.txt", "name=user\n");

    let report = apply_templates(&base, &target, &stage, &["android".into()], false).unwrap();
    let preview = &report.conflict_previews["android/config.txt"];

    assert!(preview.project_changes.contains("+ name=user"));
    assert!(preview.flutter_changes.contains("+ name=flutter"));
    assert_eq!(
        fs::read_to_string(stage.join("android/config.txt")).unwrap(),
        "name=user\n"
    );
}

#[test]
fn file_decisions_do_not_need_filesystem_access() {
    use FileDecision::{Add, Conflict, Ignore, Preserve, RemovedUpstream, Update};

    const OLD: &[u8] = b"old";
    const NEW: &[u8] = b"new";
    const CUSTOM: &[u8] = b"custom";

    let cases = [
        (Some(OLD), Some(OLD), None, Preserve),
        (Some(OLD), Some(OLD), Some(CUSTOM), Preserve),
        (Some(OLD), Some(NEW), Some(OLD), Update),
        (Some(OLD), Some(NEW), Some(NEW), Preserve),
        (Some(OLD), Some(NEW), Some(CUSTOM), Conflict),
        (Some(OLD), Some(NEW), None, Conflict),
        (None, Some(NEW), None, Add),
        (None, Some(NEW), Some(NEW), Preserve),
        (None, Some(NEW), Some(CUSTOM), Conflict),
        (Some(OLD), None, Some(OLD), RemovedUpstream),
        (Some(OLD), None, Some(CUSTOM), Preserve),
        (Some(OLD), None, None, Ignore),
        (None, None, Some(CUSTOM), Ignore),
        (None, None, None, Ignore),
    ];

    for (old, new, user, expected) in cases {
        assert_eq!(decide_file(old, new, user), expected);
    }
}

#[test]
fn independent_line_edits_merge_without_losing_user_changes() {
    let old = b"header\nold setting\nuser setting\nfooter\n";
    let new = b"header\nnew setting\nuser setting\nfooter\n";
    let user = b"header\nold setting\ncustom setting\nfooter\n";
    let expected = b"header\nnew setting\ncustom setting\nfooter\n";

    assert_eq!(merge_text(old, new, user), Some(expected.to_vec()));
    assert_eq!(
        decide_file(Some(old), Some(new), Some(user)),
        FileDecision::Merge(expected.to_vec())
    );
}

#[test]
fn overlapping_edits_and_same_position_insertions_remain_conflicts() {
    let old = b"one\ntwo\nthree\n";
    assert_eq!(
        merge_text(old, b"one\nupstream\nthree\n", b"one\nuser\nthree\n"),
        None
    );
    assert_eq!(
        merge_text(
            old,
            b"one\nupstream\ntwo\nthree\n",
            b"one\nuser\ntwo\nthree\n"
        ),
        None
    );
}

#[test]
fn identical_edits_are_not_duplicated_and_binary_files_do_not_merge() {
    let old = b"one\ntwo\nthree\n";
    let new = b"one\nupdated\nthree\n";
    assert_eq!(merge_text(old, new, new), Some(new.to_vec()));
    assert_eq!(merge_text(b"a\0b", b"c\0b", b"a\0d"), None);
}

#[test]
fn new_template_extensions_coexist_with_old_files() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    fs::create_dir(&base).unwrap();
    write(&target, "android/app/build.gradle.kts", "plugins {}\n");
    write(&stage, "android/app/build.gradle", "plugins {}\n");

    let report = apply_templates(&base, &target, &stage, &["android".into()], false).unwrap();

    assert_eq!(report.added, ["android/app/build.gradle.kts"]);
    assert!(report.conflicts.is_empty());
    assert_eq!(
        report.coexistence_notices,
        [(
            "android/app/build.gradle.kts".into(),
            "android/app/build.gradle".into()
        )]
    );
    assert!(stage.join("android/app/build.gradle.kts").exists());
    assert!(stage.join("android/app/build.gradle").exists());
}

#[test]
fn merged_file_is_written_only_to_stage() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    write(&base, "android/config.txt", "one\ntwo\nthree\n");
    write(&target, "android/config.txt", "one\nnew\nthree\n");
    write(&stage, "android/config.txt", "one\ntwo\ncustom\n");

    let report = apply_templates(&base, &target, &stage, &["android".into()], false).unwrap();

    assert_eq!(report.merged, ["android/config.txt"]);
    assert_eq!(
        fs::read_to_string(stage.join("android/config.txt")).unwrap(),
        "one\nnew\ncustom\n"
    );
    assert_eq!(
        fs::read_to_string(base.join("android/config.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
}

#[test]
fn force_overwrites_conflicting_files_with_target_template() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    write(&base, "android/config.txt", "old\n");
    write(&target, "android/config.txt", "flutter\n");
    write(&stage, "android/config.txt", "custom\n");

    let report = apply_templates(&base, &target, &stage, &["android".into()], true).unwrap();

    assert_eq!(report.forced, ["android/config.txt"]);
    assert!(report.conflicts.is_empty());
    assert_eq!(
        fs::read_to_string(stage.join("android/config.txt")).unwrap(),
        "flutter\n"
    );
}

#[test]
fn force_keeps_both_files_when_new_extension_is_introduced() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    write(&base, "android/app/build.gradle", "old plugins\n");
    write(&target, "android/app/build.gradle.kts", "plugins {}\n");
    write(&stage, "android/app/build.gradle", "groovy plugins\n");

    let report = apply_templates(&base, &target, &stage, &["android".into()], true).unwrap();

    assert_eq!(report.added, ["android/app/build.gradle.kts"]);
    assert_eq!(report.preserved, ["android/app/build.gradle"]);
    assert!(report.conflicts.is_empty());
    assert!(stage.join("android/app/build.gradle").exists());
    assert!(stage.join("android/app/build.gradle.kts").exists());
}

#[test]
fn coexistence_ignores_unrelated_prefix_matches_and_detects_format_migration() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let target = temp.path().join("target");
    let stage = temp.path().join("stage");
    fs::create_dir_all(&base).unwrap();

    // Format migration: MainActivity.kt added alongside MainActivity.java
    write(&target, "android/MainActivity.kt", "class Main");
    write(&stage, "android/MainActivity.java", "class Main");

    // Unrelated prefix: icon@2x.png added while icon.png exists
    write(&target, "android/icon@2x.png", "image2");
    write(&stage, "android/icon.png", "image1");

    let report = apply_templates(&base, &target, &stage, &["android".into()], false).unwrap();

    assert_eq!(
        report.coexistence_notices,
        [(
            "android/MainActivity.kt".into(),
            "android/MainActivity.java".into()
        )]
    );
}

#[test]
fn template_parameters_match_flutter_identifier_rules_and_instantiate_templates() {
    let info = ProjectInfo {
        name: "my_super_app".to_string(),
        revision: "rev".to_string(),
        platforms: vec!["android".into(), "ios".into()],
        organization: "com.when.in-company".to_string(),
        android_language: Some("kotlin".to_string()),
        ios_language: Some("swift".to_string()),
    };
    let params = TemplateParameters::from_project(&info);
    assert_eq!(params.project_name, "my_super_app");
    assert_eq!(params.title_case_name, "My Super App");
    assert_eq!(params.android_identifier, "com.when.incompany.my_super_app");
    assert_eq!(
        params.kotlin_android_identifier,
        "com.`when`.incompany.my_super_app"
    );
    assert_eq!(params.apple_identifier, "com.when.in-company.mySuperApp");

    let temp = tempfile::tempdir().unwrap();
    let cached = temp.path().join("cached");
    let dest = temp.path().join("dest");

    write(
        &cached,
        "android/app/src/main/kotlin/com/flup/templateplaceholder/flup_template_placeholder/MainActivity.kt",
        "package com.flup.templateplaceholder.flup_template_placeholder\n",
    );
    write(
        &cached,
        "ios/Runner.xcodeproj/project.pbxproj",
        "PRODUCT_BUNDLE_IDENTIFIER = com.flup.templateplaceholder.flupTemplatePlaceholder;\nINFOPLIST_KEY_CFBundleDisplayName = \"Flup Template Placeholder\";\n",
    );

    instantiate_cached_template(&cached, &dest, &params).unwrap();

    let kt_path =
        dest.join("android/app/src/main/kotlin/com/when/incompany/my_super_app/MainActivity.kt");
    assert_eq!(
        fs::read_to_string(kt_path).unwrap(),
        "package com.`when`.incompany.my_super_app\n"
    );

    let pbx_path = dest.join("ios/Runner.xcodeproj/project.pbxproj");
    assert_eq!(
        fs::read_to_string(pbx_path).unwrap(),
        "PRODUCT_BUNDLE_IDENTIFIER = com.when.in-company.mySuperApp;\nINFOPLIST_KEY_CFBundleDisplayName = \"My Super App\";\n"
    );
}

#[test]
fn store_cached_template_strips_non_template_entries_and_prunes_lru() {
    let temp = tempfile::tempdir().unwrap();
    let cache_root = temp.path().join("cache");
    let raw_template = temp.path().join("raw");

    write(&raw_template, "android/build.gradle", "build");
    write(&raw_template, "lib/main.dart", "void main() {}");
    write(&raw_template, "test/widget_test.dart", "void main() {}");
    write(&raw_template, "README.md", "# Readme");
    write(&raw_template, ".gitignore", "/build/");

    let entry1 = cache_root.join("entry-1");
    let entry2 = cache_root.join("entry-2");
    let entry3 = cache_root.join("entry-3");

    store_cached_template(&raw_template, &entry1, 2);
    assert!(entry1.join("android/build.gradle").exists());
    assert!(entry1.join(".gitignore").exists());
    assert!(!entry1.join("lib").exists());
    assert!(!entry1.join("test").exists());
    assert!(!entry1.join("README.md").exists());

    store_cached_template(&raw_template, &entry2, 2);
    // Touch entry1 so entry2 becomes the least-recently-used entry
    touch_cache_entry(&entry1);
    store_cached_template(&raw_template, &entry3, 2);

    prune_template_cache(&cache_root, 2);

    assert!(entry1.exists(), "recently touched entry1 should be kept");
    assert!(
        !entry2.exists(),
        "least-recently-used entry2 should be evicted"
    );
    assert!(entry3.exists(), "newest entry3 should be kept");
}
