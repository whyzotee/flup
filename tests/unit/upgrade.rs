use flup::error::{FlupError, Result};
use flup::upgrade::dependencies::DependencyResult;
use flup::upgrade::project::ProjectInfo;
use flup::upgrade::template::Report;
use flup::upgrade::{
    UpgradeBackend, UpgradeOptions, UpgradeOutcome, UpgradeStage, UpgradeWorkspace,
    upgrade_with_backend,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Default)]
struct FakeBackend {
    events: Rc<RefCell<Vec<String>>>,
    committed: Rc<RefCell<Vec<PathBuf>>>,
    conflict: bool,
    dependency_failure: bool,
    same_revision: bool,
}

struct FakeWorkspace {
    events: Rc<RefCell<Vec<String>>>,
    committed: Rc<RefCell<Vec<PathBuf>>>,
}

impl UpgradeWorkspace for FakeWorkspace {
    fn stage_path(&self) -> PathBuf {
        PathBuf::from("stage")
    }

    fn workspace_path(&self, name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    fn commit(&mut self, changed_paths: &[PathBuf]) -> Result<()> {
        self.events.borrow_mut().push("commit".into());
        self.committed.borrow_mut().extend_from_slice(changed_paths);
        Ok(())
    }
}

impl UpgradeBackend for FakeBackend {
    type Workspace = FakeWorkspace;

    fn inspect_project(&self, path: &Path) -> Result<(PathBuf, ProjectInfo)> {
        Ok((
            path.to_path_buf(),
            ProjectInfo {
                name: "sample".into(),
                revision: "base".into(),
                platforms: vec!["web".into()],
                organization: "com.example".into(),
                android_language: None,
                ios_language: None,
            },
        ))
    }

    fn resolve_sdk(&self, _: Option<&Path>) -> Result<PathBuf> {
        Ok(PathBuf::from("target-sdk"))
    }

    fn sdk_revision(&self, _: &Path) -> Result<String> {
        let revision = if self.same_revision { "base" } else { "target" };
        Ok(revision.into())
    }

    fn prepare_base_sdk(
        &self,
        _: &Path,
        _: &Path,
        _: &str,
        _: Option<&Path>,
        _: &Path,
    ) -> Result<PathBuf> {
        self.events.borrow_mut().push("prepare_base_sdk".into());
        Ok(PathBuf::from("base-sdk"))
    }

    fn begin_workspace(&self, _: &Path) -> Result<Self::Workspace> {
        self.events.borrow_mut().push("begin".into());
        Ok(FakeWorkspace {
            events: Rc::clone(&self.events),
            committed: Rc::clone(&self.committed),
        })
    }

    fn generate_template(&self, sdk: &Path, _: &Path, _: &ProjectInfo) -> Result<()> {
        self.events
            .borrow_mut()
            .push(format!("generate:{}", sdk.display()));
        Ok(())
    }

    fn apply_templates(
        &self,
        _: &Path,
        _: &Path,
        _: &Path,
        _: &[String],
        force: bool,
    ) -> Result<Report> {
        self.events.borrow_mut().push("apply_templates".into());
        let (conflicts, forced) = if self.conflict && !force {
            (vec!["web/manifest.json".into()], Vec::new())
        } else if self.conflict && force {
            (Vec::new(), vec!["web/manifest.json".into()])
        } else {
            (Vec::new(), Vec::new())
        };

        Ok(Report {
            updated: vec!["web/index.html".into()],
            merged: if self.conflict {
                Vec::new()
            } else {
                vec!["web/manifest.json".into()]
            },
            forced,
            conflicts,
            ..Report::default()
        })
    }

    fn update_metadata(&self, _: &Path, _: &str) -> Result<()> {
        self.events.borrow_mut().push("update_metadata".into());
        Ok(())
    }

    fn upgrade_dependencies(&self, _: &Path, _: &Path, _: &Path) -> Result<DependencyResult> {
        self.events.borrow_mut().push("upgrade_dependencies".into());
        if self.dependency_failure {
            return Err(FlupError::Dependency("pub failed".into()));
        }

        Ok(DependencyResult {
            output: "done".into(),
            changed_files: vec!["pubspec.lock".into()],
        })
    }
}

fn options() -> UpgradeOptions {
    UpgradeOptions {
        project: PathBuf::from("project"),
        flutter: None,
        base_flutter: None,
        no_git: false,
        force: false,
    }
}

#[test]
fn successful_upgrade_commits_only_reported_and_dependency_paths() {
    let backend = FakeBackend::default();
    let mut stages = Vec::new();
    let mut substeps = Vec::new();
    let mut reports = 0;
    let outcome = upgrade_with_backend(
        &backend,
        options(),
        |stage| match stage {
            event
            @ (UpgradeStage::SubstepStarted { .. } | UpgradeStage::SubstepFinished { .. }) => {
                substeps.push(event);
            }
            stage => stages.push(stage),
        },
        |_| {
            reports += 1;
        },
    )
    .unwrap();

    assert!(matches!(outcome, UpgradeOutcome::Completed { .. }));
    assert_eq!(reports, 1);
    assert_eq!(stages.len(), 4);
    assert!(matches!(stages[0], UpgradeStage::Staging));
    assert!(matches!(stages[3], UpgradeStage::Committing));
    assert!(matches!(
        substeps[0],
        UpgradeStage::SubstepStarted {
            id: "comparison",
            ..
        }
    ));
    assert!(matches!(
        substeps[1],
        UpgradeStage::SubstepFinished {
            id: "comparison",
            failed: false,
            ..
        }
    ));
    assert_eq!(
        &backend.events.borrow()[..2],
        &["begin", "prepare_base_sdk"]
    );
    assert_eq!(
        *backend.committed.borrow(),
        [
            "web/index.html",
            "web/manifest.json",
            ".metadata",
            "pubspec.yaml",
            "pubspec.lock"
        ]
        .map(PathBuf::from)
    );
    assert_eq!(backend.events.borrow().last().unwrap(), "commit");
}

#[test]
fn template_conflict_stops_before_metadata_dependencies_and_commit() {
    let backend = FakeBackend {
        conflict: true,
        ..FakeBackend::default()
    };
    let outcome = upgrade_with_backend(&backend, options(), |_| {}, |_| {}).unwrap();

    assert!(matches!(outcome, UpgradeOutcome::Conflicts(_)));
    assert_eq!(backend.events.borrow().last().unwrap(), "apply_templates");
    assert!(backend.committed.borrow().is_empty());
}

#[test]
fn dependency_failure_does_not_commit() {
    let backend = FakeBackend {
        dependency_failure: true,
        ..FakeBackend::default()
    };
    let error = match upgrade_with_backend(&backend, options(), |_| {}, |_| {}) {
        Ok(_) => panic!("dependency upgrade should fail"),
        Err(error) => error,
    };

    assert!(matches!(error, FlupError::Dependency(_)));
    assert_eq!(
        backend.events.borrow().last().unwrap(),
        "upgrade_dependencies"
    );
    assert!(backend.committed.borrow().is_empty());
}

#[test]
fn equal_sdk_revision_stops_before_staging() {
    let backend = FakeBackend {
        same_revision: true,
        ..FakeBackend::default()
    };
    let error = match upgrade_with_backend(&backend, options(), |_| {}, |_| {}) {
        Ok(_) => panic!("equal SDK revisions should fail"),
        Err(error) => error,
    };

    assert!(matches!(error, FlupError::Sdk(_)));
    assert!(backend.events.borrow().is_empty());
}

#[test]
fn no_git_option_skips_migration_branch() {
    let backend = FakeBackend::default();
    let mut custom_options = options();
    custom_options.no_git = true;

    let outcome = upgrade_with_backend(&backend, custom_options, |_| {}, |_| {}).unwrap();
    match outcome {
        UpgradeOutcome::Completed { git_branch, .. } => {
            assert_eq!(git_branch, None);
        }
        _ => panic!("upgrade should have completed"),
    }
}

#[test]
fn force_option_resolves_conflicts_and_commits() {
    let backend = FakeBackend {
        conflict: true,
        ..FakeBackend::default()
    };
    let mut custom_options = options();
    custom_options.force = true;

    let outcome = upgrade_with_backend(&backend, custom_options, |_| {}, |_| {}).unwrap();

    assert!(matches!(outcome, UpgradeOutcome::Completed { .. }));
    assert_eq!(backend.events.borrow().last().unwrap(), "commit");
    assert_eq!(
        *backend.committed.borrow(),
        [
            "web/index.html",
            "web/manifest.json",
            ".metadata",
            "pubspec.yaml",
            "pubspec.lock"
        ]
        .map(PathBuf::from)
    );
}
