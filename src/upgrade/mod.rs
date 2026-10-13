pub mod dependencies;
pub mod project;
pub mod runtime;
pub mod template;
pub mod text_merge;

use self::dependencies::DependencyResult;
use self::project::ProjectInfo;
use self::template::Report;
use crate::error::{FlupError, Result};
use std::path::{Path, PathBuf};

const MANDATORY_COMMIT_PATHS: [&str; 3] = [".metadata", "pubspec.yaml", "pubspec.lock"];

pub struct UpgradeOptions {
    pub project: PathBuf,
    pub flutter: Option<PathBuf>,
    pub base_flutter: Option<PathBuf>,
    pub no_git: bool,
    pub force: bool,
}

#[derive(Clone, Copy)]
pub enum UpgradeStage {
    Staging,
    GeneratingTemplates,
    UpgradingDependencies,
    Committing,
}

pub struct UpgradeSummary {
    pub project: PathBuf,
    pub base_revision: String,
    pub target_revision: String,
    pub report: Report,
}

pub enum UpgradeOutcome {
    Completed {
        summary: UpgradeSummary,
        dependency_output: String,
        dependency_files: Vec<String>,
        git_branch: Option<String>,
    },
    Conflicts(UpgradeSummary),
}

/// Operations needed by the upgrade use case. The production implementation
/// owns Flutter processes and filesystem access.
pub trait UpgradeBackend {
    type Workspace: UpgradeWorkspace;

    fn inspect_project(&self, path: &Path) -> Result<(PathBuf, ProjectInfo)>;
    fn resolve_sdk(&self, explicit: Option<&Path>) -> Result<PathBuf>;
    fn sdk_revision(&self, sdk: &Path) -> Result<String>;
    fn sdk_version(&self, sdk: &Path) -> Result<String> {
        self.sdk_revision(sdk)
    }
    fn prepare_base_sdk(
        &self,
        project: &Path,
        target_sdk: &Path,
        revision: &str,
        explicit: Option<&Path>,
        workspace: &Path,
    ) -> Result<PathBuf>;
    fn begin_workspace(&self, project: &Path) -> Result<Self::Workspace>;
    fn generate_template(&self, sdk: &Path, output: &Path, info: &ProjectInfo) -> Result<()>;
    fn generate_templates(
        &self,
        base_sdk: &Path,
        old_template: &Path,
        target_sdk: &Path,
        new_template: &Path,
        info: &ProjectInfo,
    ) -> Result<()> {
        self.generate_template(base_sdk, old_template, info)?;
        self.generate_template(target_sdk, new_template, info)?;
        Ok(())
    }
    fn apply_templates(
        &self,
        base: &Path,
        target: &Path,
        stage: &Path,
        platforms: &[String],
        force: bool,
    ) -> Result<Report>;
    fn update_metadata(&self, path: &Path, target_revision: &str) -> Result<()>;
    fn align_environment_sdk(&self, _template: &Path, _stage: &Path) -> Result<()> {
        Ok(())
    }
    fn upgrade_dependencies(
        &self,
        sdk: &Path,
        project: &Path,
        stage: &Path,
    ) -> Result<DependencyResult>;
    fn create_migration_branch(
        &self,
        _project: &Path,
        _target_version: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

pub trait UpgradeWorkspace {
    fn stage_path(&self) -> PathBuf;
    fn workspace_path(&self, name: &str) -> PathBuf;
    fn commit(&mut self, changed_paths: &[PathBuf]) -> Result<()>;
}

/// Run the upgrade using injected operations and report each stage to the caller.
pub fn upgrade_with_backend(
    backend: &impl UpgradeBackend,
    options: UpgradeOptions,
    mut on_progress: impl FnMut(UpgradeStage),
    mut on_report: impl FnMut(&UpgradeSummary),
) -> Result<UpgradeOutcome> {
    let (project, info) = backend.inspect_project(&options.project)?;
    let target_sdk = backend.resolve_sdk(options.flutter.as_deref())?;
    let target_revision = backend.sdk_revision(&target_sdk)?;
    let target_version = backend
        .sdk_version(&target_sdk)
        .unwrap_or_else(|_| target_revision.clone());

    if target_revision == info.revision {
        let message = "target Flutter SDK matches the project's base revision; nothing to upgrade";
        return Err(FlupError::Sdk(message.into()));
    }

    // Both generated templates and the dependency upgrade stay inside the workspace.
    on_progress(UpgradeStage::Staging);
    let mut transaction = backend.begin_workspace(&project)?;
    let stage = transaction.stage_path();
    let base_sdk = backend.prepare_base_sdk(
        &project,
        &target_sdk,
        &info.revision,
        options.base_flutter.as_deref(),
        &transaction.workspace_path("base-flutter-sdk"),
    )?;
    let old_template = transaction.workspace_path("base-template");
    let new_template = transaction.workspace_path("target-template");

    on_progress(UpgradeStage::GeneratingTemplates);
    backend.generate_templates(&base_sdk, &old_template, &target_sdk, &new_template, &info)?;

    let report = backend.apply_templates(
        &old_template,
        &new_template,
        &stage,
        &info.platforms,
        options.force,
    )?;
    let summary = UpgradeSummary {
        project: project.clone(),
        base_revision: info.revision,
        target_revision: target_revision.clone(),
        report,
    };
    on_report(&summary);

    // Stop before touching the original when a template has changed on both sides.
    if !summary.report.conflicts.is_empty() {
        return Ok(UpgradeOutcome::Conflicts(summary));
    }

    backend.update_metadata(&stage.join(".metadata"), &target_revision)?;
    backend.align_environment_sdk(&new_template, &stage)?;

    on_progress(UpgradeStage::UpgradingDependencies);
    let dependencies = backend.upgrade_dependencies(&target_sdk, &project, &stage)?;

    // Commit accepts paths only; reporting types stay outside the storage layer.
    let changed_paths = collect_commit_paths(&summary.report);

    on_progress(UpgradeStage::Committing);
    let git_branch = if options.no_git {
        None
    } else {
        backend.create_migration_branch(&project, &target_version)?
    };

    transaction.commit(&changed_paths)?;

    Ok(UpgradeOutcome::Completed {
        summary,
        dependency_output: dependencies.output,
        dependency_files: dependencies.changed_files,
        git_branch,
    })
}

fn collect_commit_paths(report: &Report) -> Vec<PathBuf> {
    let mut changed_paths: Vec<PathBuf> = report
        .updated
        .iter()
        .chain(&report.merged)
        .chain(&report.added)
        .chain(&report.forced)
        .chain(&report.removed)
        .map(PathBuf::from)
        .collect();

    changed_paths.extend(MANDATORY_COMMIT_PATHS.iter().map(PathBuf::from));
    changed_paths
}
