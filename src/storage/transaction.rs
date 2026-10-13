use crate::error::{FlupError, Result};
use crate::storage::commit::commit_files;
use crate::storage::tree_copy::copy_tree;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Transaction {
    project: PathBuf,
    root: PathBuf,
    keep_backup: bool,
}

impl Transaction {
    /// Create a sibling workspace with a full backup and an independent stage.
    pub fn begin(project: &Path) -> Result<Self> {
        let parent = project
            .parent()
            .ok_or_else(|| FlupError::InvalidProject("project has no parent directory".into()))?;
        let root = generate_workspace_path(parent)?;
        fs::create_dir(&root).map_err(|e| FlupError::io("cannot create temporary workspace", e))?;

        let transaction = Self {
            project: project.to_path_buf(),
            root,
            keep_backup: false,
        };

        // The stage starts from the backup, so both contain the same project snapshot.
        copy_tree(project, &transaction.backup_path())?;
        copy_tree(&transaction.backup_path(), &transaction.stage_path())?;
        Ok(transaction)
    }

    pub fn stage_path(&self) -> PathBuf {
        self.root.join("stage")
    }

    pub fn workspace_path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn commit(&mut self, changed_paths: &[PathBuf]) -> Result<()> {
        // Keep the backup if commit or rollback reports any failure.
        self.keep_backup = true;
        commit_files(
            &self.project,
            &self.stage_path(),
            &self.backup_path(),
            changed_paths,
        )?;

        self.keep_backup = false;
        Ok(())
    }

    fn backup_path(&self) -> PathBuf {
        self.root.join("backup")
    }
}

fn generate_workspace_path(parent: &Path) -> Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            let message = format!("system time is before UNIX epoch: {error}");
            FlupError::Internal(message)
        })?
        .as_nanos();
    Ok(parent.join(format!(".flup-temp-{}-{stamp}", std::process::id())))
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.keep_backup {
            eprintln!("flup: temporary backup retained at {}", self.root.display());
            return;
        }

        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!(
                "flup: could not remove temporary workspace {}: {error}",
                self.root.display()
            );
        }
    }
}
