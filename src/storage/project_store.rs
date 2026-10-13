use crate::error::{FlupError, Result};
use crate::storage::fs_util::read_optional_text;
use crate::upgrade::project::{
    ProjectInfo, align_environment_sdk_content, project_name, project_organization,
    project_revision, update_metadata_content,
};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SUPPORTED_PLATFORMS: [&str; 6] = ["android", "ios", "linux", "macos", "windows", "web"];
const NATIVE_PLATFORMS: [&str; 3] = ["android", "ios", "macos"];
const IDENTIFIER_FILE_PATHS: [&str; 5] = [
    "android/app/build.gradle.kts",
    "android/app/build.gradle",
    "android/app/src/main/AndroidManifest.xml",
    "ios/Runner.xcodeproj/project.pbxproj",
    "macos/Runner.xcodeproj/project.pbxproj",
];

pub fn inspect_project(path: &Path) -> Result<(PathBuf, ProjectInfo)> {
    let project = fs::canonicalize(path).map_err(|error| {
        let context = format!("cannot open project {}", path.display());
        FlupError::io(context, error)
    })?;
    if !project.is_dir() {
        return Err(FlupError::InvalidProject(
            "project path is not a directory".into(),
        ));
    }

    Ok((project.clone(), read_project(&project)?))
}

fn read_project(project: &Path) -> Result<ProjectInfo> {
    let pubspec = read_pubspec(project)?;
    let name = project_name(&pubspec)?;

    let metadata = read_metadata(project)?;
    let revision = project_revision(&metadata)?;

    let platforms = detect_platforms(project)?;
    let sources = collect_identifier_sources(project)?;
    let has_native = has_native_platform(project);
    let organization = project_organization(&name, &sources, has_native)?;
    let android_language = detect_android_language(project);
    let ios_language = detect_ios_language(project);

    Ok(ProjectInfo {
        name,
        revision,
        platforms,
        organization,
        android_language,
        ios_language,
    })
}

fn read_pubspec(project: &Path) -> Result<String> {
    fs::read_to_string(project.join("pubspec.yaml"))
        .map_err(|e| FlupError::io("pubspec.yaml is missing or unreadable", e))
}

fn read_metadata(project: &Path) -> Result<String> {
    fs::read_to_string(project.join(".metadata")).map_err(|e| {
        FlupError::io(
            ".metadata is missing or unreadable; cannot identify the base revision",
            e,
        )
    })
}

fn detect_platforms(project: &Path) -> Result<Vec<String>> {
    let platforms: Vec<String> = SUPPORTED_PLATFORMS
        .into_iter()
        .filter(|platform| project.join(platform).is_dir())
        .map(str::to_string)
        .collect();

    if platforms.is_empty() {
        return Err(FlupError::InvalidProject(
            "no supported Flutter platform directory found".into(),
        ));
    }

    Ok(platforms)
}

fn collect_identifier_sources(project: &Path) -> Result<Vec<String>> {
    // Native identifiers let both SDKs generate templates with the same organization.
    let sources: Vec<String> = IDENTIFIER_FILE_PATHS
        .iter()
        .map(|path| read_optional_text(&project.join(path)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    Ok(sources)
}

fn has_native_platform(project: &Path) -> bool {
    NATIVE_PLATFORMS
        .iter()
        .any(|platform| project.join(platform).exists())
}

fn detect_android_language(project: &Path) -> Option<String> {
    if !project.join("android").is_dir() {
        return None;
    }

    let main_dir = project.join("android/app/src/main");
    if !main_dir.is_dir() {
        return Some("kotlin".into());
    }

    if main_dir.join("kotlin").is_dir() {
        return Some("kotlin".into());
    }

    let mut has_java = false;
    let mut has_kt = false;

    for entry in WalkDir::new(&main_dir).max_depth(10).into_iter().flatten() {
        if entry.file_type().is_file() {
            match entry.path().extension().and_then(|e| e.to_str()) {
                Some("kt") => {
                    has_kt = true;
                    break;
                }
                Some("java") => {
                    has_java = true;
                }
                _ => {}
            }
        }
    }

    let language = if has_kt {
        "kotlin"
    } else if has_java {
        "java"
    } else {
        "kotlin"
    };

    Some(language.into())
}

fn detect_ios_language(project: &Path) -> Option<String> {
    if !project.join("ios").is_dir() {
        return None;
    }

    let runner = project.join("ios/Runner");
    let mut has_swift = false;
    let mut has_objc = false;

    if runner.is_dir() {
        for entry in WalkDir::new(&runner).max_depth(5).into_iter().flatten() {
            if entry.file_type().is_file() {
                match entry.path().extension().and_then(|e| e.to_str()) {
                    Some("swift") => {
                        has_swift = true;
                        break;
                    }
                    Some("m" | "mm") => {
                        has_objc = true;
                    }
                    _ => {}
                }
            }
        }
    }

    let language = if has_swift {
        "swift"
    } else if has_objc {
        "objc"
    } else {
        "swift"
    };

    Some(language.into())
}

pub fn update_metadata(path: &Path, target_revision: &str) -> Result<()> {
    let content =
        fs::read_to_string(path).map_err(|e| FlupError::io("cannot read .metadata", e))?;
    let updated = update_metadata_content(&content, target_revision)?;
    fs::write(path, updated).map_err(|e| FlupError::io("cannot update .metadata", e))
}

pub fn align_environment_sdk(template_dir: &Path, stage_dir: &Path) -> Result<()> {
    let template_pubspec_path = template_dir.join("pubspec.yaml");
    let stage_pubspec_path = stage_dir.join("pubspec.yaml");

    if !template_pubspec_path.is_file() || !stage_pubspec_path.is_file() {
        return Ok(());
    }

    let template_pubspec = fs::read_to_string(&template_pubspec_path)
        .map_err(|e| FlupError::io("cannot read template pubspec.yaml", e))?;
    let stage_pubspec = fs::read_to_string(&stage_pubspec_path)
        .map_err(|e| FlupError::io("cannot read stage pubspec.yaml", e))?;

    if let Some(updated_content) = align_environment_sdk_content(&template_pubspec, &stage_pubspec)?
    {
        fs::write(&stage_pubspec_path, updated_content)
            .map_err(|e| FlupError::io("cannot update environment.sdk in pubspec.yaml", e))?;
    }

    Ok(())
}
