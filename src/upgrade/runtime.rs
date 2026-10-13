use crate::error::{FlupError, Result};
use crate::flutter;
use crate::flutter::process;
use crate::storage::project_store;
use crate::storage::template_store;
use crate::storage::transaction::Transaction;
use crate::upgrade::dependencies::{self, DependencyResult};
use crate::upgrade::project::ProjectInfo;
use crate::upgrade::template::Report;
use crate::upgrade::{
    UpgradeBackend, UpgradeOptions, UpgradeOutcome, UpgradeStage, UpgradeSummary, UpgradeWorkspace,
    upgrade_with_backend,
};
use std::path::{Path, PathBuf};

pub struct SystemUpgradeBackend;

pub fn upgrade(
    options: UpgradeOptions,
    on_progress: impl FnMut(UpgradeStage),
    on_report: impl FnMut(&UpgradeSummary),
) -> Result<UpgradeOutcome> {
    upgrade_with_backend(&SystemUpgradeBackend, options, on_progress, on_report)
}

impl UpgradeBackend for SystemUpgradeBackend {
    type Workspace = Transaction;

    fn inspect_project(&self, path: &Path) -> Result<(PathBuf, ProjectInfo)> {
        project_store::inspect_project(path)
    }

    fn resolve_sdk(&self, explicit: Option<&Path>) -> Result<PathBuf> {
        flutter::resolve_sdk(explicit)
    }

    fn sdk_revision(&self, sdk: &Path) -> Result<String> {
        process::sdk_revision(sdk)
    }

    fn prepare_base_sdk(
        &self,
        project: &Path,
        target_sdk: &Path,
        revision: &str,
        explicit: Option<&Path>,
        workspace: &Path,
    ) -> Result<PathBuf> {
        flutter::prepare_base_sdk(project, target_sdk, revision, explicit, workspace)
    }

    fn begin_workspace(&self, project: &Path) -> Result<Self::Workspace> {
        Transaction::begin(project)
    }

    fn generate_template(&self, sdk: &Path, output: &Path, info: &ProjectInfo) -> Result<()> {
        process::generate_template(sdk, output, info)
    }

    fn generate_templates(
        &self,
        base_sdk: &Path,
        old_template: &Path,
        target_sdk: &Path,
        new_template: &Path,
        info: &ProjectInfo,
    ) -> Result<()> {
        std::thread::scope(|s| {
            let base_handle = s.spawn(|| process::generate_template(base_sdk, old_template, info));
            let target_handle =
                s.spawn(|| process::generate_template(target_sdk, new_template, info));

            let base_res = base_handle.join().map_err(|_| {
                FlupError::Internal("base template generation thread panicked".into())
            })?;
            let target_res = target_handle.join().map_err(|_| {
                FlupError::Internal("target template generation thread panicked".into())
            })?;

            base_res?;
            target_res?;
            Ok(())
        })
    }

    fn apply_templates(
        &self,
        base: &Path,
        target: &Path,
        stage: &Path,
        platforms: &[String],
        force: bool,
    ) -> Result<Report> {
        template_store::apply_templates(base, target, stage, platforms, force)
    }

    fn update_metadata(&self, path: &Path, target_revision: &str) -> Result<()> {
        project_store::update_metadata(path, target_revision)
    }

    fn align_environment_sdk(&self, template: &Path, stage: &Path) -> Result<()> {
        project_store::align_environment_sdk(template, stage)
    }

    fn sdk_version(&self, sdk: &Path) -> Result<String> {
        process::sdk_version(sdk)
    }

    fn upgrade_dependencies(
        &self,
        sdk: &Path,
        project: &Path,
        stage: &Path,
    ) -> Result<DependencyResult> {
        dependencies::upgrade_dependencies(sdk, project, stage)
    }

    fn create_migration_branch(
        &self,
        project: &Path,
        target_version: &str,
    ) -> Result<Option<String>> {
        process::create_migration_branch(project, target_version)
    }
}

impl UpgradeWorkspace for Transaction {
    fn stage_path(&self) -> PathBuf {
        Transaction::stage_path(self)
    }

    fn workspace_path(&self, name: &str) -> PathBuf {
        Transaction::workspace_path(self, name)
    }

    fn commit(&mut self, changed_paths: &[PathBuf]) -> Result<()> {
        Transaction::commit(self, changed_paths)
    }
}
