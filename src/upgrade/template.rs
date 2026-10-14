use super::project::ProjectInfo;
use super::text_merge::merge_text;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CANONICAL_PROJECT_NAME: &str = "flup_template_placeholder";
pub const CANONICAL_ORGANIZATION: &str = "com.flup.templateplaceholder";
const RESERVED_KOTLIN_KEYWORDS: [&str; 3] = ["when", "in", "is"];

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

/// Derived identifiers and naming variants used by `flutter create --template=app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateParameters {
    pub project_name: String,
    pub organization: String,
    pub title_case_name: String,
    pub android_identifier: String,
    pub kotlin_android_identifier: String,
    pub android_package_path: String,
    pub apple_identifier: String,
}

impl TemplateParameters {
    pub fn new(project_name: &str, organization: &str) -> Self {
        let title_case_name = snake_case_to_title_case(project_name);
        let android_identifier = create_android_identifier(organization, project_name);
        let kotlin_android_identifier = escape_kotlin_keywords(&android_identifier);
        let android_package_path = android_identifier.replace('.', "/");
        let apple_identifier = create_uti_identifier(organization, project_name);

        Self {
            project_name: project_name.to_string(),
            organization: organization.to_string(),
            title_case_name,
            android_identifier,
            kotlin_android_identifier,
            android_package_path,
            apple_identifier,
        }
    }

    pub fn canonical() -> Self {
        Self::new(CANONICAL_PROJECT_NAME, CANONICAL_ORGANIZATION)
    }

    pub fn from_project(info: &ProjectInfo) -> Self {
        Self::new(&info.name, &info.organization)
    }

    /// Map a relative path from a canonical cached template to the target project's layout.
    pub fn instantiate_relative_path(&self, relative: &Path) -> PathBuf {
        let canonical = Self::canonical();
        if *self == canonical {
            return relative.to_path_buf();
        }

        let normalized = relative.to_string_lossy().replace('\\', "/");
        if !normalized.contains("flup") {
            return relative.to_path_buf();
        }

        let mapped = normalized
            .replace(&canonical.android_package_path, &self.android_package_path)
            .replace(&canonical.project_name, &self.project_name);

        PathBuf::from(mapped)
    }

    /// Replace canonical placeholder tokens in UTF-8 template files while leaving binary files intact.
    pub fn instantiate_content(&self, relative_path: &Path, content: &[u8]) -> Vec<u8> {
        if content.contains(&0) {
            return content.to_vec();
        }

        let Ok(text) = std::str::from_utf8(content) else {
            return content.to_vec();
        };

        if !text.contains("flup") && !text.contains("Flup") {
            return content.to_vec();
        }

        let canonical = Self::canonical();
        if *self == canonical {
            return content.to_vec();
        }

        let is_kotlin = relative_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "kt");
        let target_android_id = if is_kotlin {
            &self.kotlin_android_identifier
        } else {
            &self.android_identifier
        };

        let rendered = text
            .replace(&canonical.android_package_path, &self.android_package_path)
            .replace(&canonical.android_identifier, target_android_id)
            .replace(&canonical.apple_identifier, &self.apple_identifier)
            .replace(&canonical.organization, &self.organization)
            .replace(&canonical.title_case_name, &self.title_case_name)
            .replace(&canonical.project_name, &self.project_name);

        rendered.into_bytes()
    }
}

/// Build a canonical `ProjectInfo` so cached templates do not depend on project name or organization.
pub fn canonical_project_info(info: &ProjectInfo) -> ProjectInfo {
    ProjectInfo {
        name: CANONICAL_PROJECT_NAME.into(),
        revision: info.revision.clone(),
        platforms: info.platforms.clone(),
        organization: CANONICAL_ORGANIZATION.into(),
        android_language: info.android_language.clone(),
        ios_language: info.ios_language.clone(),
    }
}

/// Matches `camelCase` in `flutter_tools/lib/src/base/utils.dart`.
pub fn flutter_camel_case(input: &str) -> String {
    let mut current = input.to_string();
    while let Some(byte_idx) = current.find('_') {
        let char_idx = current[..byte_idx].chars().count();
        let total_chars = current.chars().count();
        if total_chars < 2 || char_idx >= total_chars - 2 {
            break;
        }

        let mut chars: Vec<char> = current.chars().collect();
        chars.remove(char_idx);
        if let Some(next_char) = chars.get_mut(char_idx) {
            *next_char = next_char.to_ascii_uppercase();
        }
        current = chars.into_iter().collect();
    }
    current
}

/// Matches `snakeCaseToTitleCase` in `flutter_tools/lib/src/base/utils.dart`.
pub fn snake_case_to_title_case(snake_case: &str) -> String {
    snake_case
        .split('_')
        .map(|segment| sentence_case(&flutter_camel_case(segment)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sentence_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut result = first.to_uppercase().to_string();
            result.push_str(chars.as_str());
            result
        }
    }
}

/// Matches `CreateBase.createAndroidIdentifier` in `flutter_tools`.
pub fn create_android_identifier(organization: &str, name: &str) -> String {
    let raw = format!("{organization}.{name}");
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect();

    let mut segments: Vec<String> = cleaned
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect();

    while segments.len() < 2 {
        segments.push("untitled".into());
    }

    for segment in &mut segments {
        let starts_with_ascii_letter = segment
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic());
        if !starts_with_ascii_letter {
            segment.insert(0, 'u');
        }
    }

    segments.join(".")
}

/// Matches `_escapeKotlinKeywords` in `flutter_tools/lib/src/template.dart`.
pub fn escape_kotlin_keywords(android_identifier: &str) -> String {
    android_identifier
        .split('.')
        .map(|segment| {
            if RESERVED_KOTLIN_KEYWORDS.contains(&segment) {
                format!("`{segment}`")
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Matches `CreateBase.createUTIIdentifier` in `flutter_tools`.
pub fn create_uti_identifier(organization: &str, name: &str) -> String {
    let camel_name = flutter_camel_case(name);
    let raw = format!("{organization}.{camel_name}");
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            c.is_ascii_alphanumeric()
                || *c == '-'
                || *c == '.'
                || ('\u{0080}'..='\u{ffff}').contains(c)
        })
        .collect();

    let mut segments: Vec<String> = cleaned
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect();

    while segments.len() < 2 {
        segments.push("untitled".into());
    }

    segments.join(".")
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
