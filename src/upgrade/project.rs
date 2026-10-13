use crate::error::{FlupError, Result};
use regex::Regex;
use std::sync::LazyLock;
use yaml_rust2::{Yaml, YamlLoader};

static IDENTIFIER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:(?:namespace|applicationId|package)\s*(?:=\s*)?["']|PRODUCT_BUNDLE_IDENTIFIER\s*=\s*)([A-Za-z][A-Za-z0-9_.-]+)"#,
    )
    .expect("valid identifier regex pattern")
});

#[derive(Debug)]
pub struct ProjectInfo {
    pub name: String,
    pub revision: String,
    pub platforms: Vec<String>,
    pub organization: String,
    pub android_language: Option<String>,
    pub ios_language: Option<String>,
}

pub fn project_name(pubspec: &str) -> Result<String> {
    let document = parse_document(pubspec, "pubspec.yaml")?;
    validate_supported_pubspec_features(&document)?;
    extract_project_name(&document)
}

fn extract_project_name(document: &Yaml) -> Result<String> {
    document["name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| FlupError::InvalidProject("pubspec.yaml has no name".into()))
}

fn validate_supported_pubspec_features(document: &Yaml) -> Result<()> {
    if has_workspace_configuration(document) || has_path_dependency(document) {
        let message = "path dependencies and pub workspaces are not yet supported because staging changes their resolution paths";
        return Err(FlupError::InvalidProject(message.into()));
    }
    Ok(())
}

fn has_workspace_configuration(document: &Yaml) -> bool {
    let has_workspace_key = !matches!(document["workspace"], Yaml::BadValue);
    let uses_workspace_resolution = document["resolution"].as_str() == Some("workspace");
    has_workspace_key || uses_workspace_resolution
}

fn has_path_dependency(document: &Yaml) -> bool {
    // A temporary sibling copy changes how relative path dependencies resolve.
    const SECTIONS: [&str; 3] = ["dependencies", "dev_dependencies", "dependency_overrides"];
    for section in SECTIONS {
        if let Some(dependencies) = document[section].as_hash() {
            for spec in dependencies.values() {
                if !matches!(spec["path"], Yaml::BadValue) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn project_revision(metadata: &str) -> Result<String> {
    let document = parse_document(metadata, ".metadata")?;
    validate_project_type(&document)?;
    let revision = extract_project_revision(&document)?;
    validate_aligned_platform_revisions(&document, &revision)?;
    Ok(revision)
}

fn validate_project_type(document: &Yaml) -> Result<()> {
    let project_type = document["project_type"].as_str().unwrap_or_default();
    if project_type != "app" {
        let message = format!("only Flutter app projects are supported (found {project_type})");
        return Err(FlupError::InvalidProject(message));
    }
    Ok(())
}

fn extract_project_revision(document: &Yaml) -> Result<String> {
    let revision_node = if matches!(document["version"], Yaml::BadValue) {
        &document["revision"]
    } else {
        &document["version"]["revision"]
    };

    revision_node
        .as_str()
        .map(str::to_string)
        .filter(|revision| is_git_revision(revision))
        .ok_or_else(|| FlupError::InvalidProject(".metadata has no valid Flutter revision".into()))
}

fn validate_aligned_platform_revisions(document: &Yaml, expected_revision: &str) -> Result<()> {
    // Different platform base revisions cannot be reproduced by one old SDK.
    if is_divergent_base_revision(&document["base_revision"], expected_revision) {
        return Err(divergent_platform_error());
    }

    if let Some(platforms) = document["migration"]["platforms"].as_vec() {
        for platform in platforms {
            if is_divergent_base_revision(&platform["base_revision"], expected_revision) {
                return Err(divergent_platform_error());
            }
        }
    }

    Ok(())
}

fn divergent_platform_error() -> FlupError {
    let message = "platforms have different base revisions; upgrade them separately after aligning their SDK revisions";
    FlupError::InvalidProject(message.into())
}

fn is_git_revision(revision: &str) -> bool {
    revision.len() >= 12
        && revision
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn is_divergent_base_revision(base: &Yaml, revision: &str) -> bool {
    let is_missing = matches!(base, Yaml::BadValue | Yaml::Null);
    !is_missing && base.as_str() != Some(revision)
}

fn parse_document(input: &str, file: &'static str) -> Result<Yaml> {
    let mut documents = YamlLoader::load_from_str(input)
        .map_err(|source| FlupError::InvalidYaml { file, source })?;
    let has_one_mapping = documents.len() == 1 && documents[0].as_hash().is_some();

    if !has_one_mapping {
        let message = format!("{file} must contain one YAML mapping");
        return Err(FlupError::InvalidProject(message));
    }

    Ok(documents.remove(0))
}

pub fn project_organization(
    name: &str,
    sources: &[String],
    has_native_platform: bool,
) -> Result<String> {
    let name_suffix = format!(".{name}");
    let name_lower = format!(".{}", name.to_lowercase());

    let identifiers: Vec<String> = sources
        .iter()
        .filter_map(|content| {
            let id = IDENTIFIER_PATTERN.captures(content)?.get(1)?.as_str();

            // 1. Exact suffix match (.name)
            if let Some(org) = id.strip_suffix(&name_suffix) {
                return Some(org.to_string());
            }

            // 2. Case-insensitive suffix match
            if id.to_lowercase().ends_with(&name_lower) {
                let end_idx = id.len() - name_lower.len();
                return Some(id[..end_idx].to_string());
            }

            // 3. Fallback: take prefix before the last dot
            if let Some((org, _)) = id.rsplit_once('.').filter(|(org, _)| !org.is_empty()) {
                return Some(org.to_string());
            }

            None
        })
        .collect();

    if identifiers.is_empty() {
        if !has_native_platform {
            return Ok("com.example".into());
        }

        let message = "cannot infer the original organization identifier; no matching Android or Apple identifier found";
        return Err(FlupError::InvalidProject(message.into()));
    }

    let identifiers_disagree = identifiers.iter().any(|id| id != &identifiers[0]);
    if identifiers_disagree {
        let message =
            "Android and Apple organization identifiers disagree; cannot safely recreate templates";
        return Err(FlupError::InvalidProject(message.into()));
    }

    Ok(identifiers[0].clone())
}

/// Replace Flutter revisions in metadata while keeping unrelated fields intact.
pub fn update_metadata_content(content: &str, target_revision: &str) -> Result<String> {
    parse_document(content, ".metadata")?;

    let mut updated = String::new();
    let mut sections: Vec<(usize, &str)> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if is_blank_or_comment(trimmed) {
            push_line(&mut updated, line);
            continue;
        }

        let indent = line.len() - trimmed.len();
        pop_deeper_sections(&mut sections, indent);

        let Some((raw_key, _)) = trimmed.split_once(':') else {
            push_line(&mut updated, line);
            continue;
        };

        let key = clean_yaml_key(raw_key);
        if is_target_revision_field(key, &sections) {
            let prefix = &line[..indent];
            updated.push_str(&format!("{prefix}{key}: {target_revision}\n"));
        } else {
            push_line(&mut updated, line);
        }

        sections.push((indent, key));
    }

    Ok(updated)
}

fn is_blank_or_comment(trimmed: &str) -> bool {
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn pop_deeper_sections(sections: &mut Vec<(usize, &str)>, indent: usize) {
    while sections.last().is_some_and(|(level, _)| *level >= indent) {
        sections.pop();
    }
}

fn clean_yaml_key(raw_key: &str) -> &str {
    raw_key.trim_start_matches("- ").trim()
}

fn is_target_revision_field(key: &str, sections: &[(usize, &str)]) -> bool {
    let is_root_field = sections.is_empty();
    let is_version_field = matches!(sections, [(_, "version")]);
    let is_platform_field = matches!(sections, [(_, "migration"), (_, "platforms"), ..]);

    let is_revision = key == "revision" && (is_root_field || is_version_field);
    let is_base_revision = key == "base_revision" && (is_root_field || is_platform_field);

    is_revision || is_base_revision
}

fn push_line(buffer: &mut String, line: &str) {
    buffer.push_str(line);
    buffer.push('\n');
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

pub fn parse_min_sdk_version(constraint: &str) -> Option<SemVer> {
    static VER_REGEX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\d+)\.(\d+)(?:\.(\d+))?").expect("valid semver regex"));

    let captures = VER_REGEX.captures(constraint)?;
    let major = captures.get(1)?.as_str().parse().ok()?;
    let minor = captures.get(2)?.as_str().parse().ok()?;
    let patch = captures
        .get(3)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);

    Some(SemVer {
        major,
        minor,
        patch,
    })
}

pub fn extract_environment_sdk(pubspec: &str) -> Option<String> {
    let documents = YamlLoader::load_from_str(pubspec).ok()?;
    let doc = documents.first()?;
    doc["environment"]["sdk"].as_str().map(str::to_string)
}

pub fn align_environment_sdk_content(
    template_pubspec: &str,
    project_pubspec: &str,
) -> Result<Option<String>> {
    let target_constraint = match extract_environment_sdk(template_pubspec) {
        Some(c) => c,
        None => return Ok(None),
    };
    let current_constraint = match extract_environment_sdk(project_pubspec) {
        Some(c) => c,
        None => return Ok(None),
    };

    let target_min = match parse_min_sdk_version(&target_constraint) {
        Some(v) => v,
        None => return Ok(None),
    };
    let current_min = match parse_min_sdk_version(&current_constraint) {
        Some(v) => v,
        None => return Ok(None),
    };

    if current_min < target_min {
        let updated = update_pubspec_environment_sdk(project_pubspec, &target_constraint)?;
        Ok(Some(updated))
    } else {
        Ok(None)
    }
}

pub fn update_pubspec_environment_sdk(content: &str, new_constraint: &str) -> Result<String> {
    parse_document(content, "pubspec.yaml")?;

    let formatted_constraint = if new_constraint.contains(['<', '>'])
        && !new_constraint.starts_with('\'')
        && !new_constraint.starts_with('"')
    {
        format!("'{new_constraint}'")
    } else {
        new_constraint.to_string()
    };

    let mut updated = String::new();
    let mut sections: Vec<(usize, &str)> = Vec::new();
    let mut replaced = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        if is_blank_or_comment(trimmed) {
            push_line(&mut updated, line);
            continue;
        }

        let indent = line.len() - trimmed.len();
        pop_deeper_sections(&mut sections, indent);

        let Some((raw_key, rest)) = trimmed.split_once(':') else {
            push_line(&mut updated, line);
            continue;
        };

        let key = clean_yaml_key(raw_key);
        let in_environment = sections
            .last()
            .is_some_and(|(_, sec)| *sec == "environment")
            && indent > sections.last().unwrap().0;

        if in_environment && key == "sdk" && !replaced {
            let prefix = &line[..indent];
            let comment = rest
                .split_once('#')
                .map(|(_, c)| format!(" #{c}"))
                .unwrap_or_default();
            updated.push_str(&format!("{prefix}sdk: {formatted_constraint}{comment}\n"));
            replaced = true;
        } else {
            push_line(&mut updated, line);
        }

        sections.push((indent, key));
    }

    Ok(updated)
}
