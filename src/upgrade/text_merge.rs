const MAX_DIFF_CELLS: usize = 1_000_000;

/// Merge independent line edits made against the same template revision.
/// Ambiguous edits, binary files, and unusually large diffs stay conflicts.
pub fn merge_text(base: &[u8], upstream: &[u8], user: &[u8]) -> Option<Vec<u8>> {
    if [base, upstream, user]
        .iter()
        .any(|content| !is_text(content))
    {
        return None;
    }

    let base_lines: Vec<&[u8]> = base.split_inclusive(|byte| *byte == b'\n').collect();
    let upstream_lines: Vec<&[u8]> = upstream.split_inclusive(|byte| *byte == b'\n').collect();
    let user_lines: Vec<&[u8]> = user.split_inclusive(|byte| *byte == b'\n').collect();

    let user_edits = diff(&base_lines, &user_lines, MAX_DIFF_CELLS)?;
    let upstream_edits = diff(&base_lines, &upstream_lines, MAX_DIFF_CELLS)?;
    let mut edits = user_edits;

    for incoming in upstream_edits {
        if edits.iter().any(|edit| edit == &incoming) {
            continue;
        }

        if edits.iter().any(|edit| edit.overlaps(&incoming)) {
            return None;
        }

        edits.push(incoming);
    }

    edits.sort_by_key(|edit| (edit.start, edit.end));
    let mut merged = Vec::new();
    let mut cursor = 0;

    for edit in edits {
        for line in &base_lines[cursor..edit.start] {
            merged.extend_from_slice(line);
        }

        for line in edit.replacement {
            merged.extend_from_slice(line);
        }

        cursor = edit.end;
    }

    for line in &base_lines[cursor..] {
        merged.extend_from_slice(line);
    }

    Some(merged)
}

/// Show a small, bounded preview of changes relative to the old template.
pub fn diff_preview(base: &[u8], changed: &[u8]) -> Option<String> {
    const MAX_PREVIEW_LINES: usize = 12;

    if !is_text(base) || !is_text(changed) {
        return None;
    }

    let base_lines: Vec<&[u8]> = base.split_inclusive(|byte| *byte == b'\n').collect();
    let changed_lines: Vec<&[u8]> = changed.split_inclusive(|byte| *byte == b'\n').collect();
    let edits = diff(&base_lines, &changed_lines, MAX_DIFF_CELLS)?;
    let mut preview = String::new();
    let mut shown = 0;

    for edit in edits {
        for (prefix, line) in base_lines[edit.start..edit.end]
            .iter()
            .map(|line| ('-', *line))
            .chain(edit.replacement.into_iter().map(|line| ('+', line)))
        {
            if shown == MAX_PREVIEW_LINES {
                preview.push_str("    ... more changes omitted\n");
                return Some(preview);
            }

            let text = std::str::from_utf8(line).ok()?;
            let trimmed = text.trim_end_matches(['\r', '\n']);
            let short: String = trimmed.chars().take(160).collect();
            preview.push_str(&format!("    {prefix} {short}\n"));
            shown += 1;
        }
    }

    Some(preview)
}

fn is_text(content: &[u8]) -> bool {
    !content.contains(&0) && std::str::from_utf8(content).is_ok()
}

#[derive(PartialEq, Eq)]
struct Edit<'a> {
    start: usize,
    end: usize,
    replacement: Vec<&'a [u8]>,
}

impl Edit<'_> {
    fn overlaps(&self, other: &Self) -> bool {
        if self.start == self.end {
            return other.start <= self.start && self.start <= other.end;
        }

        if other.start == other.end {
            return self.start <= other.start && other.start <= self.end;
        }

        self.start < other.end && other.start < self.end
    }
}

fn diff<'a>(base: &[&[u8]], changed: &[&'a [u8]], limit: usize) -> Option<Vec<Edit<'a>>> {
    let mut prefix = 0;
    while prefix < base.len() && prefix < changed.len() && base[prefix] == changed[prefix] {
        prefix += 1;
    }

    if prefix == base.len() && prefix == changed.len() {
        return Some(Vec::new());
    }

    let mut suffix = 0;
    while suffix < (base.len() - prefix)
        && suffix < (changed.len() - prefix)
        && base[base.len() - 1 - suffix] == changed[changed.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mid_base = &base[prefix..base.len() - suffix];
    let mid_changed = &changed[prefix..changed.len() - suffix];

    if mid_base.is_empty() {
        return Some(vec![Edit {
            start: prefix,
            end: prefix,
            replacement: mid_changed.to_vec(),
        }]);
    }

    if mid_changed.is_empty() {
        return Some(vec![Edit {
            start: prefix,
            end: prefix + mid_base.len(),
            replacement: Vec::new(),
        }]);
    }

    let mut edits = diff_lcs(mid_base, mid_changed, limit)?;
    for edit in &mut edits {
        edit.start += prefix;
        edit.end += prefix;
    }

    Some(edits)
}

fn diff_lcs<'a>(base: &[&[u8]], changed: &[&'a [u8]], limit: usize) -> Option<Vec<Edit<'a>>> {
    let rows = base.len().checked_add(1)?;
    let columns = changed.len().checked_add(1)?;
    if rows.checked_mul(columns)? > limit {
        return None;
    }

    let mut common = vec![0usize; rows * columns];
    for base_index in (0..base.len()).rev() {
        for changed_index in (0..changed.len()).rev() {
            let index = base_index * columns + changed_index;
            common[index] = if base[base_index] == changed[changed_index] {
                common[(base_index + 1) * columns + changed_index + 1] + 1
            } else {
                common[(base_index + 1) * columns + changed_index]
                    .max(common[base_index * columns + changed_index + 1])
            };
        }
    }

    let mut edits = Vec::new();
    let (mut base_index, mut changed_index) = (0, 0);
    let mut current: Option<Edit<'a>> = None;

    while base_index < base.len() || changed_index < changed.len() {
        let lines_match = base_index < base.len()
            && changed_index < changed.len()
            && base[base_index] == changed[changed_index];

        if lines_match {
            if let Some(edit) = current.take() {
                edits.push(edit);
            }
            base_index += 1;
            changed_index += 1;
            continue;
        }

        let edit = current.get_or_insert_with(|| Edit {
            start: base_index,
            end: base_index,
            replacement: Vec::new(),
        });

        let remove_base_line = base_index < base.len()
            && (changed_index == changed.len()
                || common[(base_index + 1) * columns + changed_index]
                    >= common[base_index * columns + changed_index + 1]);

        if remove_base_line {
            base_index += 1;
            edit.end = base_index;
        } else {
            edit.replacement.push(changed[changed_index]);
            changed_index += 1;
        }
    }

    if let Some(edit) = current {
        edits.push(edit);
    }

    Some(edits)
}
