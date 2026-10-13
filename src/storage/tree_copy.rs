use crate::error::{FlupError, Result};
use std::fs;
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination).map_err(|e| FlupError::io("cannot create stage", e))?;

    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry = entry.map_err(|error| {
            let message = format!("cannot copy project tree: {error}");
            FlupError::InvalidProject(message)
        })?;
        let relative = entry.path().strip_prefix(source).map_err(|error| {
            let message = format!("project entry is outside root: {error}");
            FlupError::Internal(message)
        })?;
        let target = destination.join(relative);

        copy_entry(&entry, &target)?;
    }
    Ok(())
}

fn copy_entry(entry: &DirEntry, target: &Path) -> Result<()> {
    // A link in the snapshot could redirect template or dependency writes.
    if entry.file_type().is_symlink() {
        let message = format!(
            "symbolic link {} cannot be safely staged",
            entry.path().display()
        );
        return Err(FlupError::InvalidProject(message));
    }

    if entry.file_type().is_dir() {
        fs::create_dir(target).map_err(|error| {
            let context = format!("cannot create {}", target.display());
            FlupError::io(context, error)
        })?;
    } else if entry.file_type().is_file() {
        fs::copy(entry.path(), target).map_err(|error| {
            let context = format!("cannot copy {}", target.display());
            FlupError::io(context, error)
        })?;
    } else {
        let message = format!("unsupported file type: {}", entry.path().display());
        return Err(FlupError::InvalidProject(message));
    }

    Ok(())
}
