use super::text_merge::merge_text;
use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct ConflictPreview {
    pub project_changes: String,
    pub flutter_changes: String,
}

#[derive(Default, Debug)]
pub struct Report {
    pub updated: Vec<String>,
    pub merged: Vec<String>,
    pub added: Vec<String>,
    pub forced: Vec<String>,
    pub removed: Vec<String>,
    pub removed_upstream: Vec<String>,
    pub preserved: Vec<String>,
    pub conflicts: Vec<String>,
    pub coexistence_notices: Vec<(String, String)>,
    pub conflict_previews: BTreeMap<String, ConflictPreview>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileDecision {
    Update,
    Merge(Vec<u8>),
    Add,
    RemovedUpstream,
    Preserve,
    Conflict,
    Ignore,
}

/// Decide one template file's outcome without reading or writing the filesystem.
pub fn decide_file(old: Option<&[u8]>, new: Option<&[u8]>, user: Option<&[u8]>) -> FileDecision {
    match (old, new) {
        (Some(old), Some(new)) => decide_existing_file(old, new, user),
        (None, Some(new)) => decide_added_file(new, user),
        (Some(old), None) => decide_removed_file(old, user),
        (None, None) => FileDecision::Ignore,
    }
}

fn decide_existing_file(old: &[u8], new: &[u8], user: Option<&[u8]>) -> FileDecision {
    // No upstream change means there is nothing to apply, even for a customized file.
    if old == new {
        return FileDecision::Preserve;
    }

    match user {
        Some(user) if user == old => FileDecision::Update,
        Some(user) if user == new => FileDecision::Preserve,
        Some(user) => merge_text(old, new, user)
            .map(FileDecision::Merge)
            .unwrap_or(FileDecision::Conflict),
        _ => FileDecision::Conflict,
    }
}

fn decide_added_file(new: &[u8], user: Option<&[u8]>) -> FileDecision {
    match user {
        None => FileDecision::Add,
        Some(user) if user == new => FileDecision::Preserve,
        Some(_) => FileDecision::Conflict,
    }
}

fn decide_removed_file(old: &[u8], user: Option<&[u8]>) -> FileDecision {
    // Upstream removals are reported, never applied to the user's project.
    match user {
        Some(user) if user == old => FileDecision::RemovedUpstream,
        Some(_) => FileDecision::Preserve,
        None => FileDecision::Ignore,
    }
}
