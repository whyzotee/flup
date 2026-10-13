use crate::error::{FlupError, Result};
use std::fs;
use std::path::Path;

/// Read a file's raw bytes if it exists, returning `Ok(None)` if not found.
pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            let context = format!("cannot read {}", path.display());
            Err(FlupError::io(context, error))
        }
    }
}

/// Read a file's UTF-8 text if it exists, returning `Ok(None)` if not found.
pub fn read_optional_text(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            let context = format!("cannot read {}", path.display());
            Err(FlupError::io(context, error))
        }
    }
}
