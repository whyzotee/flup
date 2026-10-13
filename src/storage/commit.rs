use crate::error::{FlupError, Result};
use crate::storage::fs_util::read_optional;
use std::collections::BTreeSet;
use std::fs::{self, Permissions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use tempfile::NamedTempFile;

enum PendingOperation {
    Write {
        relative: PathBuf,
        content: Vec<u8>,
        permissions: Permissions,
    },
    Delete {
        relative: PathBuf,
    },
}

impl PendingOperation {
    fn relative(&self) -> &Path {
        match self {
            Self::Write { relative, .. } => relative,
            Self::Delete { relative } => relative,
        }
    }
}

/// Apply selected staged files and restore earlier writes if a later write fails.
pub fn commit_files(
    project: &Path,
    stage: &Path,
    backup: &Path,
    changed_paths: &[PathBuf],
) -> Result<()> {
    // Validate every requested write before changing any original file.
    let operations = collect_pending_operations(project, stage, backup, changed_paths)?;
    apply_pending_operations(project, backup, operations)
}

fn collect_pending_operations(
    project: &Path,
    stage: &Path,
    backup: &Path,
    changed_paths: &[PathBuf],
) -> Result<Vec<PendingOperation>> {
    let changed: BTreeSet<&PathBuf> = changed_paths.iter().collect();
    let mut operations = Vec::new();
    for relative in changed {
        if let Some(operation) = prepare_operation(project, stage, backup, relative)? {
            operations.push(operation);
        }
    }
    Ok(operations)
}

fn apply_pending_operations(
    project: &Path,
    backup: &Path,
    operations: Vec<PendingOperation>,
) -> Result<()> {
    // A failed write restores only files already applied; the backup remains available.
    let mut applied = Vec::new();
    for operation in operations {
        let relative = operation.relative().to_path_buf();
        if let Err(source) = apply_operation(project, backup, &operation) {
            let restore_errors = rollback(project, backup, &applied);

            let restore_status = if restore_errors.is_empty() {
                "original files were restored".to_string()
            } else {
                format!("restore errors: {}", restore_errors.join(", "))
            };

            return Err(FlupError::CommitFailed {
                source: Box::new(source),
                restore_status,
                backup: backup.to_path_buf(),
            });
        }

        applied.push(relative);
    }

    Ok(())
}

fn prepare_operation(
    project: &Path,
    stage: &Path,
    backup: &Path,
    relative: &Path,
) -> Result<Option<PendingOperation>> {
    validate_relative(relative)?;
    check_project_path(project, relative)?;

    let staged = stage.join(relative);
    let staged_metadata = match fs::symlink_metadata(&staged) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            let context = format!("cannot inspect {}", staged.display());
            return Err(FlupError::io(context, error));
        }
    };

    match staged_metadata {
        Some(metadata) => {
            if !metadata.file_type().is_file() {
                let message = format!(
                    "staged file is missing or is not a regular file: {}",
                    relative.display()
                );
                return Err(FlupError::Commit(message));
            }

            let content = fs::read(&staged).map_err(|error| {
                let context = format!("cannot read {}", staged.display());
                FlupError::io(context, error)
            })?;

            let current = ensure_unmodified_during_upgrade(
                project,
                backup,
                relative,
                "; no files were committed",
            )?;

            if current.as_ref() == Some(&content) {
                return Ok(None);
            }

            Ok(Some(PendingOperation::Write {
                relative: relative.to_path_buf(),
                content,
                permissions: metadata.permissions(),
            }))
        }
        None => {
            let current = ensure_unmodified_during_upgrade(
                project,
                backup,
                relative,
                "; no files were committed",
            )?;

            if current.is_none() {
                return Ok(None);
            }

            Ok(Some(PendingOperation::Delete {
                relative: relative.to_path_buf(),
            }))
        }
    }
}

fn ensure_unmodified_during_upgrade(
    project: &Path,
    backup: &Path,
    relative: &Path,
    error_suffix: &str,
) -> Result<Option<Vec<u8>>> {
    let project_path = project.join(relative);
    let backup_path = backup.join(relative);

    let project_len = fs::metadata(&project_path).ok().map(|m| m.len());
    let backup_len = fs::metadata(&backup_path).ok().map(|m| m.len());

    if project_len != backup_len {
        let message = format!(
            "project changed during upgrade at {}{error_suffix}",
            relative.display()
        );
        return Err(FlupError::Commit(message));
    }

    let current = read_optional(&project_path)?;
    let saved = read_optional(&backup_path)?;
    if current != saved {
        let message = format!(
            "project changed during upgrade at {}{error_suffix}",
            relative.display()
        );
        return Err(FlupError::Commit(message));
    }
    Ok(current)
}

fn validate_relative(relative: &Path) -> Result<()> {
    let is_empty = relative.as_os_str().is_empty();
    let has_invalid_component = relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)));

    if is_empty || has_invalid_component {
        let message = format!(
            "invalid relative path in transaction: {}",
            relative.display()
        );
        return Err(FlupError::Commit(message));
    }
    Ok(())
}

fn check_project_path(project: &Path, relative: &Path) -> Result<()> {
    // Reject symlink components so writes cannot follow paths outside the project.
    let mut path = project.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        path.push(component.as_os_str());
        let is_leaf = index + 1 == component_count;
        validate_path_component(&path, relative, is_leaf)?;
    }
    Ok(())
}

fn validate_path_component(path: &Path, relative: &Path, is_leaf: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let message = format!(
                "project path became a symbolic link: {}",
                relative.display()
            );
            Err(FlupError::Commit(message))
        }
        Ok(metadata) if !is_leaf && !metadata.is_dir() => {
            let message = format!("project path is not a directory: {}", path.display());
            Err(FlupError::Commit(message))
        }
        Ok(metadata) if is_leaf && !metadata.is_file() => {
            let message = format!("project path is not a regular file: {}", path.display());
            Err(FlupError::Commit(message))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            let context = format!("cannot inspect {}", path.display());
            Err(FlupError::io(context, error))
        }
    }
}

fn apply_operation(project: &Path, backup: &Path, operation: &PendingOperation) -> Result<()> {
    match operation {
        PendingOperation::Write {
            relative,
            content,
            permissions,
        } => {
            let destination = project.join(relative);
            let parent = destination
                .parent()
                .ok_or_else(|| FlupError::Commit("commit destination has no parent".into()))?;
            fs::create_dir_all(parent).map_err(|error| {
                let context = format!("cannot create {}", parent.display());
                FlupError::io(context, error)
            })?;

            // Recheck immediately before replacement in case the project changed during staging.
            check_project_path(project, relative)?;
            ensure_unmodified_during_upgrade(project, backup, relative, "")?;

            replace_file(&destination, content, permissions.clone())
        }
        PendingOperation::Delete { relative } => {
            let destination = project.join(relative);
            check_project_path(project, relative)?;
            ensure_unmodified_during_upgrade(project, backup, relative, "")?;

            remove_file_if_exists(&destination)
        }
    }
}

fn rollback(project: &Path, backup: &Path, applied: &[PathBuf]) -> Vec<String> {
    let mut errors = Vec::new();

    for relative in applied.iter().rev() {
        if let Err(error) = rollback_file(project, backup, relative) {
            errors.push(format!("{}: {error}", relative.display()));
        }
    }

    errors
}

fn rollback_file(project: &Path, backup: &Path, relative: &Path) -> Result<()> {
    check_project_path(project, relative)?;

    let destination = project.join(relative);
    let saved = backup.join(relative);

    if let Some(content) = read_optional(&saved)? {
        let metadata = fs::metadata(&saved).map_err(|error| {
            let context = format!("cannot inspect {}", saved.display());
            FlupError::io(context, error)
        })?;

        let permissions = metadata.permissions();

        return replace_file(&destination, &content, permissions);
    }

    remove_file_if_exists(&destination)
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            let context = format!("cannot remove {}", path.display());
            Err(FlupError::io(context, error))
        }
    }
}

fn replace_file(destination: &Path, content: &[u8], permissions: Permissions) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| FlupError::Commit("commit destination has no parent".into()))?;

    // Place the temporary file beside its destination for an atomic replacement.
    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
        FlupError::io(
            format!("cannot create temporary file in {}", parent.display()),
            error,
        )
    })?;

    temporary.write_all(content).map_err(|error| {
        FlupError::io(
            format!("cannot write temporary file for {}", destination.display()),
            error,
        )
    })?;

    temporary
        .as_file()
        .set_permissions(permissions)
        .map_err(|error| {
            FlupError::io(
                format!("cannot set permissions for {}", destination.display()),
                error,
            )
        })?;

    temporary.persist(destination).map(|_| ()).map_err(|error| {
        FlupError::io(
            format!("cannot replace {}", destination.display()),
            error.error,
        )
    })
}
