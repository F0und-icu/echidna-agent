//! The deliberately small, text-only unified Diff subset for `apply_patch`.

#[derive(Clone)]
pub(super) struct ParsedPatch {
    pub(super) path: String,
    hunks: Vec<ParsedHunk>,
}

#[derive(Clone)]
struct ParsedHunk {
    old_start: usize,
    new_start: usize,
    old_count: usize,
    new_count: usize,
    lines: Vec<PatchLine>,
}

#[derive(Clone)]
enum PatchLine {
    Context(String),
    Remove(String),
    Add(String),
}

pub(super) fn parse_patch(patch: &str) -> Result<Vec<ParsedPatch>, String> {
    if patch.is_empty()
        || !patch.ends_with('\n')
        || patch.contains('\r')
        || patch.contains("\\ No newline at end of file")
    {
        return Err(
            "patch must be non-empty, LF-only, newline-terminated, and must not use no-newline markers"
                .to_owned(),
        );
    }
    let lines = patch.split_terminator('\n').collect::<Vec<_>>();
    let mut index = 0;
    let mut files = Vec::new();
    while index < lines.len() {
        let old_header = lines[index];
        let Some(old_path) = old_header.strip_prefix("--- a/") else {
            return Err("each file patch must start with `--- a/<relative-path>`".to_owned());
        };
        index += 1;
        let new_header = lines
            .get(index)
            .ok_or_else(|| "patch is missing its `+++` header".to_owned())?;
        let Some(new_path) = new_header.strip_prefix("+++ b/") else {
            return Err("each file patch must use `+++ b/<relative-path>`".to_owned());
        };
        if old_path != new_path || old_path.is_empty() {
            return Err(
                "this minimal Patch subset supports only one in-place file replacement".to_owned(),
            );
        }
        index += 1;
        let mut hunks = Vec::new();
        while index < lines.len() && lines[index].starts_with("@@ ") {
            let header = lines[index];
            index += 1;
            let (old_start, old_count, new_start, new_count) = parse_hunk_header(header)?;
            let mut hunk_lines = Vec::new();
            let mut old_seen = 0_usize;
            let mut new_seen = 0_usize;
            while old_seen < old_count || new_seen < new_count {
                let line = *lines
                    .get(index)
                    .ok_or_else(|| "hunk ended before its declared line counts".to_owned())?;
                index += 1;
                let (prefix, content) = line
                    .split_at_checked(1)
                    .ok_or_else(|| "hunk lines must begin with space, `+`, or `-`".to_owned())?;
                let parsed = match prefix {
                    " " => {
                        old_seen += 1;
                        new_seen += 1;
                        PatchLine::Context(content.to_owned())
                    }
                    "-" => {
                        old_seen += 1;
                        PatchLine::Remove(content.to_owned())
                    }
                    "+" => {
                        new_seen += 1;
                        PatchLine::Add(content.to_owned())
                    }
                    _ => return Err("hunk lines must begin with space, `+`, or `-`".to_owned()),
                };
                if old_seen > old_count || new_seen > new_count {
                    return Err("hunk has more lines than declared by its header".to_owned());
                }
                hunk_lines.push(parsed);
            }
            hunks.push(ParsedHunk {
                old_start,
                new_start,
                old_count,
                new_count,
                lines: hunk_lines,
            });
        }
        if hunks.is_empty() {
            return Err("a file patch must contain at least one hunk".to_owned());
        }
        files.push(ParsedPatch {
            path: old_path.to_owned(),
            hunks,
        });
    }
    if files.len() != 1 {
        return Err("this Slice accepts exactly one file per apply_patch call".to_owned());
    }
    Ok(files)
}

fn parse_hunk_header(value: &str) -> Result<(usize, usize, usize, usize), String> {
    let rest = value
        .strip_prefix("@@ -")
        .and_then(|value| value.split_once(" +"))
        .ok_or_else(|| "invalid unified Diff hunk header".to_owned())?;
    let old = parse_range(rest.0)?;
    let (new_range, suffix) = rest
        .1
        .split_once(" @@")
        .ok_or_else(|| "invalid unified Diff hunk header".to_owned())?;
    if !suffix.is_empty() && !suffix.starts_with(' ') {
        return Err("invalid unified Diff hunk suffix".to_owned());
    }
    let new = parse_range(new_range)?;
    if old.0 == 0 || old.1 == 0 || new.0 == 0 || new.1 == 0 {
        return Err("this minimal Patch subset does not support added or deleted files".to_owned());
    }
    Ok((old.0, old.1, new.0, new.1))
}

fn parse_range(value: &str) -> Result<(usize, usize), String> {
    let (start, count) = match value.split_once(',') {
        Some((start, count)) => (start, count),
        None => (value, "1"),
    };
    let start = start
        .parse::<usize>()
        .map_err(|_| "invalid hunk line number".to_owned())?;
    let count = count
        .parse::<usize>()
        .map_err(|_| "invalid hunk line count".to_owned())?;
    Ok((start, count))
}

pub(super) fn apply_hunks(
    before: &Option<Vec<u8>>,
    patch: &ParsedPatch,
) -> Result<Vec<u8>, String> {
    let bytes = before
        .as_deref()
        .ok_or_else(|| "Patch target does not exist".to_owned())?;
    let source =
        std::str::from_utf8(bytes).map_err(|_| "Patch target must be UTF-8 text".to_owned())?;
    if !source.ends_with('\n') || source.contains('\r') {
        return Err("this minimal Patch subset requires LF-terminated text".to_owned());
    }
    let source_lines = source.split_terminator('\n').collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut cursor = 0_usize;
    for hunk in &patch.hunks {
        let start = hunk.old_start.saturating_sub(1);
        if start < cursor || start > source_lines.len() {
            return Err("hunk starts outside the source or overlaps a prior hunk".to_owned());
        }
        output.extend(
            source_lines[cursor..start]
                .iter()
                .map(|line| (*line).to_owned()),
        );
        if hunk.new_start - 1 != output.len() {
            return Err("hunk new start does not match the resulting line position".to_owned());
        }
        let mut old_seen = 0_usize;
        let mut new_seen = 0_usize;
        for line in &hunk.lines {
            match line {
                PatchLine::Context(expected) => {
                    let actual = source_lines
                        .get(start + old_seen)
                        .ok_or_else(|| "context extends past source".to_owned())?;
                    if actual != expected {
                        return Err("context preimage does not match source".to_owned());
                    }
                    output.push(expected.clone());
                    old_seen += 1;
                    new_seen += 1;
                }
                PatchLine::Remove(expected) => {
                    let actual = source_lines
                        .get(start + old_seen)
                        .ok_or_else(|| "removal extends past source".to_owned())?;
                    if actual != expected {
                        return Err("removal preimage does not match source".to_owned());
                    }
                    old_seen += 1;
                }
                PatchLine::Add(value) => {
                    output.push(value.clone());
                    new_seen += 1;
                }
            }
        }
        if old_seen != hunk.old_count || new_seen != hunk.new_count {
            return Err("hunk line counts do not match its header".to_owned());
        }
        cursor = start + old_seen;
    }
    output.extend(source_lines[cursor..].iter().map(|line| (*line).to_owned()));
    Ok(format!("{}\n", output.join("\n")).into_bytes())
}
