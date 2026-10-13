use crate::error::{FlupError, Result};
use crate::flutter::process::run_flutter;
use crate::storage::fs_util::read_optional;
use std::path::Path;

const DEPENDENCY_FILES: [&str; 2] = ["pubspec.yaml", "pubspec.lock"];

pub struct DependencyResult {
    pub output: String,
    pub changed_files: Vec<String>,
}

pub fn upgrade_dependencies(sdk: &Path, project: &Path, stage: &Path) -> Result<DependencyResult> {
    let output = run_flutter(sdk, stage, &["pub", "upgrade", "--major-versions"])?;
    validate_lockfile_exists(stage)?;

    let changed_files = changed_dependency_files(project, stage)?;
    Ok(DependencyResult {
        output,
        changed_files,
    })
}

pub fn changed_dependency_files(project: &Path, stage: &Path) -> Result<Vec<String>> {
    let mut changed_files = Vec::new();
    for file in DEPENDENCY_FILES {
        let original = read_optional(&project.join(file))?;
        let upgraded = read_optional(&stage.join(file))?;
        if original != upgraded {
            changed_files.push(file.to_string());
        }
    }

    Ok(changed_files)
}

fn validate_lockfile_exists(stage: &Path) -> Result<()> {
    if !stage.join("pubspec.lock").is_file() {
        return Err(FlupError::Dependency(
            "flutter pub upgrade did not produce pubspec.lock".into(),
        ));
    }
    Ok(())
}
