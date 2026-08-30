use std::collections::BTreeSet;

const MAX_TERMS: usize = 256;
const MAX_NGRAM_CHARS: usize = 768;

pub(super) fn normalized_terms(text: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for raw in text.split_whitespace() {
        let value = normalize(raw);
        if looks_like_identifier(raw, &value) {
            continue;
        }
        for term in raw
            .split(|character: char| !character.is_alphanumeric())
            .map(normalize)
            .filter(|term| term.len() >= 2)
        {
            terms.insert(term);
        }
        if terms.len() >= MAX_TERMS {
            break;
        }
    }
    terms.into_iter().collect()
}

pub(super) fn identifiers(text: &str) -> Vec<String> {
    let mut values = BTreeSet::new();
    for raw in text.split_whitespace() {
        let raw = raw.trim_matches(|character: char| {
            matches!(
                character,
                ',' | ';' | '(' | ')' | '{' | '}' | '[' | ']' | '"' | '\''
            )
        });
        if raw.is_empty() {
            continue;
        }
        let value = normalize(raw);
        if looks_like_identifier(raw, &value) {
            values.insert(value.clone());
            add_url_components(&value, &mut values);
            add_delimited_components(&value, &mut values);
        }
        if values.len() >= MAX_TERMS {
            break;
        }
    }
    values.into_iter().collect()
}

/// Returns only the complete structured identifiers present in the input.
///
/// `identifiers` also exposes safe subcomponents for lexical ranking, but a
/// component such as `target` must not turn a different package or class into
/// an exact identifier hit.
pub(super) fn exact_identifiers(text: &str) -> Vec<String> {
    let mut values = BTreeSet::new();
    for raw in text.split_whitespace() {
        let raw = raw.trim_matches(|character: char| {
            matches!(
                character,
                ',' | ';' | '(' | ')' | '{' | '}' | '[' | ']' | '"' | '\''
            )
        });
        let value = normalize(raw);
        if !value.is_empty() && looks_like_identifier(raw, &value) {
            values.insert(value);
        }
        if values.len() >= MAX_TERMS {
            break;
        }
    }
    values.into_iter().collect()
}

pub(super) fn ngrams(text: &str) -> BTreeSet<String> {
    let lexical_text = text
        .split_whitespace()
        .filter_map(|raw| {
            let value = normalize(raw);
            (!is_strict_security_identifier(&value)).then_some(value)
        })
        .collect::<Vec<_>>()
        .join(" ");
    let characters = lexical_text
        .chars()
        .filter(|character| !character.is_whitespace())
        .take(MAX_NGRAM_CHARS)
        .collect::<Vec<_>>();
    let mut grams = BTreeSet::new();
    for width in [2, 3] {
        for window in characters.windows(width) {
            grams.insert(window.iter().collect());
        }
    }
    grams
}

pub(super) fn ngram_similarity(
    query: &BTreeSet<String>,
    candidate: &BTreeSet<String>,
) -> Option<f64> {
    if query.is_empty() || candidate.is_empty() {
        return None;
    }
    let intersection = query.intersection(candidate).count();
    let union = query.union(candidate).count();
    let minimum_overlap = if query.len() <= 2 { 1 } else { 2 };
    let query_coverage = intersection as f64 / query.len() as f64;
    (intersection >= minimum_overlap && query_coverage >= 0.4 && union > 0)
        .then_some(intersection as f64 / union as f64)
}

pub(super) fn normalize(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

fn looks_like_identifier(raw: &str, value: &str) -> bool {
    let strict_security_identifier = is_strict_security_identifier(value);
    let structured = value.chars().any(|character| {
        matches!(
            character,
            '.' | '_' | '-' | '/' | ':' | '?' | '&' | '=' | '#' | '@' | '$'
        )
    });
    let symbol = looks_like_symbol(raw);
    strict_security_identifier || structured || symbol
}

fn is_strict_security_identifier(value: &str) -> bool {
    value.starts_with("cve-")
        || value.starts_with("cwe-")
        || (value.len() >= 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (value.starts_with("0x")
            && value.len() > 3
            && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn looks_like_symbol(raw: &str) -> bool {
    let mut saw_letter = false;
    let mut saw_uppercase = false;
    let mut saw_lowercase = false;
    let mut uppercase_after_first_letter = false;
    for character in raw.chars().filter(|character| character.is_alphabetic()) {
        if saw_letter && character.is_uppercase() {
            uppercase_after_first_letter = true;
        }
        saw_letter = true;
        saw_uppercase |= character.is_uppercase();
        saw_lowercase |= character.is_lowercase();
    }
    (saw_uppercase && !saw_lowercase) || (saw_lowercase && uppercase_after_first_letter)
}

fn add_url_components(value: &str, values: &mut BTreeSet<String>) {
    let Some((scheme, remainder)) = value.split_once("://") else {
        return;
    };
    if !scheme.is_empty() {
        values.insert(scheme.to_owned());
    }
    let (host, path_and_query) = remainder.split_once('/').unwrap_or((remainder, ""));
    if !host.is_empty() {
        values.insert(host.to_owned());
    }
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    for part in path.split('/').filter(|part| !part.is_empty()) {
        values.insert(part.to_owned());
    }
    for parameter in query.split('&').filter(|parameter| !parameter.is_empty()) {
        let key = parameter.split_once('=').map_or(parameter, |(key, _)| key);
        if !key.is_empty() {
            values.insert(key.to_owned());
        }
    }
}

fn add_delimited_components(value: &str, values: &mut BTreeSet<String>) {
    for part in value.split(['.', '/', ':', '?', '&', '=']) {
        if part.len() >= 2 {
            values.insert(part.to_owned());
        }
    }
}
