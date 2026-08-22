//! Deterministic complete replacement Diffs for local file audit facts.

/// Produces self-contained, line-oriented evidence for one completed file
/// replacement. The caller supplies labels because this is an audit rendering
/// rule, not a Patch parser.
pub(crate) fn replacement_diff(
    old: Option<&str>,
    new: Option<&str>,
    old_label: &str,
    new_label: &str,
) -> String {
    let mut output = format!("--- {old_label}\n+++ {new_label}\n");
    for line in old.unwrap_or_default().split_inclusive('\n') {
        output.push('-');
        output.push_str(line);
        if !line.ends_with('\n') {
            output.push('\n');
            output.push_str("\\ No newline at end of file\n");
        }
    }
    for line in new.unwrap_or_default().split_inclusive('\n') {
        output.push('+');
        output.push_str(line);
        if !line.ends_with('\n') {
            output.push('\n');
            output.push_str("\\ No newline at end of file\n");
        }
    }
    output
}
