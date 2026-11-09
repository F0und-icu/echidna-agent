use std::collections::{BTreeMap, BTreeSet};

const MAX_TERMS: usize = 256;
pub(super) const MAX_NGRAM_CHARS: usize = 768;

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

pub(super) fn task_single_character_query(query: &str) -> Option<String> {
    let query = query.trim();
    let mut characters = query.chars();
    characters.next()?;
    characters.next().is_none().then(|| normalize(query))
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
    let mut characters = Vec::new();
    for raw in text.split_whitespace() {
        let value = normalize(raw);
        if !is_strict_security_identifier(&value) {
            characters.extend(
                value
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .take(MAX_NGRAM_CHARS - characters.len()),
            );
            if characters.len() == MAX_NGRAM_CHARS {
                break;
            }
        }
    }
    let mut grams = BTreeSet::new();
    for width in [2, 3] {
        for window in characters.windows(width) {
            grams.insert(window.iter().collect());
        }
    }
    grams
}

struct TaskNgramToken {
    start: usize,
    end: usize,
    character_base: usize,
    characters: Vec<(char, usize)>,
    non_hex_suffix: Vec<usize>,
    strict: bool,
}

impl TaskNgramToken {
    fn strict_suffix(&self, start: usize) -> bool {
        self.strict_range(start, self.characters.len())
    }

    fn strict_range(&self, start: usize, end: usize) -> bool {
        let rest = &self.characters[start..end];
        let starts = |prefix: &str| {
            rest.iter()
                .map(|(character, _)| *character)
                .zip(prefix.chars())
                .all(|(left, right)| left == right)
                && rest.len() >= prefix.chars().count()
        };
        starts("cve-")
            || starts("cwe-")
            || (rest.len() >= 32 && self.non_hex_suffix[start] == self.non_hex_suffix[end])
            || (rest.len() > 3
                && starts("0x")
                && self.non_hex_suffix[start + 2] == self.non_hex_suffix[end])
    }
}

/// Query-gram occurrence upper bound for every suffix of one bounded source
/// window. Strict identifiers may be skipped after the first token, while a
/// suffix beginning inside one may expose that token's remaining characters.
pub(super) struct TaskNgramIndex {
    tokens: Vec<TaskNgramToken>,
    filtered: Vec<(char, usize)>,
    query_ids: BTreeMap<String, usize>,
    occurrences: Vec<(usize, usize)>,
    token_grams: [Vec<(usize, usize)>; 2],
    filtered_grams: [Vec<(usize, usize)>; 2],
    local: SlidingGramSet,
    window: SlidingGramSet,
    boundary: Vec<usize>,
    source_len: usize,
}

type TaskNgramSegment<'a> = (&'a [(char, usize)], usize, usize, usize, usize);

// The ranges address sorted occurrence arrays. Updating only their changed
// portions avoids scanning every distinct query gram at every source anchor.
struct SlidingGramSet {
    ranges: Vec<(usize, usize)>,
    counts: Vec<usize>,
    overlap: usize,
}

impl SlidingGramSet {
    fn new(slots: usize, grams: usize) -> Self {
        Self {
            ranges: vec![(0, 0); slots],
            counts: vec![0; grams],
            overlap: 0,
        }
    }

    fn add(&mut self, id: usize) {
        self.overlap += usize::from(self.counts[id] == 0);
        self.counts[id] += 1;
    }

    fn remove(&mut self, id: usize) {
        self.counts[id] -= 1;
        self.overlap -= usize::from(self.counts[id] == 0);
    }

    fn update(
        &mut self,
        slot: usize,
        events: &[(usize, usize)],
        start: usize,
        end: usize,
        probes: &mut usize,
    ) {
        let from = events.partition_point(|(position, _)| *position < start);
        let to = events
            .partition_point(|(position, _)| *position < end)
            .max(from);
        let (old_from, old_to) = self.ranges[slot];
        for &(_, id) in events[old_from..old_to.min(from).max(old_from)]
            .iter()
            .chain(events[old_from.max(to).min(old_to)..old_to].iter())
        {
            self.remove(id);
            *probes += 1;
        }
        for &(_, id) in events[from..to.min(old_from).max(from)]
            .iter()
            .chain(events[from.max(old_to).min(to)..to].iter())
        {
            self.add(id);
            *probes += 1;
        }
        self.ranges[slot] = (from, to);
    }
}

impl TaskNgramIndex {
    pub(super) fn new(text: &str, query: &BTreeSet<String>) -> Self {
        let mut tokens = Vec::new();
        let mut filtered = Vec::new();
        let query_ids = query
            .iter()
            .cloned()
            .enumerate()
            .map(|(id, gram)| (gram, id))
            .collect::<BTreeMap<_, _>>();
        let mut token_grams = [Vec::new(), Vec::new()];
        let mut character_base = 0;
        let mut scanned = 0;
        for raw in text.split_whitespace() {
            let start = scanned + text[scanned..].find(raw).expect("split token in source");
            scanned = start + raw.len();
            let characters = raw
                .char_indices()
                .flat_map(|(offset, character)| {
                    character
                        .to_lowercase()
                        .filter(|value| !value.is_whitespace())
                        .map(move |value| (value, start + offset))
                })
                .collect::<Vec<_>>();
            let value = characters
                .iter()
                .map(|(character, _)| *character)
                .collect::<String>();
            let strict = is_strict_security_identifier(&value);
            if !strict {
                filtered.extend_from_slice(&characters);
            }
            record_task_grams(&characters, character_base, &query_ids, &mut token_grams);
            let mut non_hex_suffix = vec![0; characters.len() + 1];
            for index in (0..characters.len()).rev() {
                non_hex_suffix[index] = non_hex_suffix[index + 1]
                    + usize::from(!characters[index].0.is_ascii_hexdigit());
            }
            tokens.push(TaskNgramToken {
                start,
                end: scanned,
                character_base,
                characters,
                non_hex_suffix,
                strict,
            });
            character_base += tokens.last().unwrap().characters.len();
        }
        let mut filtered_grams = [Vec::new(), Vec::new()];
        record_task_grams(&filtered, 0, &query_ids, &mut filtered_grams);
        let mut following = Vec::new();
        let mut occurrences = Vec::new();
        for token in tokens.iter().rev() {
            let characters = &token.characters;
            let mut boundary = characters[characters.len().saturating_sub(2)..].to_vec();
            let tail_len = boundary.len();
            boundary.extend(&following);
            for width in [2, 3] {
                for window in boundary
                    .windows(width)
                    .take(tail_len)
                    .rev()
                    .chain(characters.windows(width).rev())
                {
                    let gram = window
                        .iter()
                        .map(|(character, _)| *character)
                        .collect::<String>();
                    if let Some(&id) = query_ids.get(&gram) {
                        occurrences.push((window[0].1, id));
                    }
                }
            }
            if !token.strict {
                let mut prefix = characters.iter().take(2).copied().collect::<Vec<_>>();
                prefix.extend(&following);
                prefix.truncate(2);
                following = prefix;
            }
        }
        occurrences.sort_unstable();
        occurrences.dedup();
        Self {
            tokens,
            filtered,
            query_ids,
            occurrences,
            token_grams,
            filtered_grams,
            local: SlidingGramSet::new(6, query.len()),
            window: SlidingGramSet::new(1, query.len()),
            boundary: Vec::new(),
            source_len: text.len(),
        }
    }

    pub(super) fn may_match(
        &mut self,
        query: &BTreeSet<String>,
        start: usize,
        probes: &mut usize,
    ) -> bool {
        let Some(token) = self
            .tokens
            .get(self.tokens.partition_point(|token| token.end <= start))
        else {
            return false;
        };
        let first = token
            .characters
            .partition_point(|(_, offset)| *offset < start);
        let first_is_strict = token.strict_suffix(first);
        let first_len = if first_is_strict {
            0
        } else {
            token.characters.len() - first
        };
        let possible_start = if first_is_strict { token.end } else { start };
        let end = if first_len >= MAX_NGRAM_CHARS {
            token.characters[first + MAX_NGRAM_CHARS - 1].1
        } else {
            let next = self
                .filtered
                .partition_point(|(_, offset)| *offset < token.end);
            let needed = MAX_NGRAM_CHARS - first_len;
            self.filtered
                .get(next + needed - 1)
                .map(|(_, offset)| *offset)
                .unwrap_or(self.source_len)
        };
        self.window.update(
            0,
            &self.occurrences,
            possible_start,
            end.saturating_add(1),
            probes,
        );
        ngram_overlap_matches(query.len(), self.window.overlap)
    }

    /// Exact query-gram intersection for the local source slice, without
    /// renormalizing that slice for each candidate anchor.
    pub(super) fn local_ngrams(&mut self, start: usize, end: usize, probes: &mut usize) -> usize {
        let first = self.tokens.partition_point(|token| token.end <= start);
        let after_last = self.tokens.partition_point(|token| token.start < end);
        if first >= after_last {
            for slot in 0..6 {
                let postings = if slot / 2 == 1 {
                    &self.filtered_grams[slot % 2]
                } else {
                    &self.token_grams[slot % 2]
                };
                self.local.update(slot, postings, 0, 0, probes);
            }
            for id in self.boundary.drain(..) {
                self.local.remove(id);
                *probes += 1;
            }
            return 0;
        }
        let last = after_last - 1;
        let mut segments: Vec<TaskNgramSegment<'_>> = Vec::new();
        let first_token = &self.tokens[first];
        let first_start = first_token
            .characters
            .partition_point(|(_, offset)| *offset < start);
        let first_end = if first == last {
            first_token
                .characters
                .partition_point(|(_, offset)| *offset < end)
        } else {
            first_token.characters.len()
        };
        if !first_token.strict_range(first_start, first_end) {
            segments.push((
                &first_token.characters,
                first_start,
                first_end,
                first_token.character_base,
                0,
            ));
        }
        if first != last {
            let last_token = &self.tokens[last];
            let middle_start = self
                .filtered
                .partition_point(|(_, offset)| *offset < first_token.end);
            let middle_end = self
                .filtered
                .partition_point(|(_, offset)| *offset < last_token.start);
            segments.push((&self.filtered, middle_start, middle_end, 0, 1));
            let last_end = last_token
                .characters
                .partition_point(|(_, offset)| *offset < end);
            if !last_token.strict_range(0, last_end) {
                segments.push((
                    &last_token.characters,
                    0,
                    last_end,
                    last_token.character_base,
                    2,
                ));
            }
        }
        for id in self.boundary.drain(..) {
            self.local.remove(id);
            *probes += 1;
        }
        let mut ranges = [(0, 0); 6];
        let mut remaining = MAX_NGRAM_CHARS;
        let mut tail = Vec::new();
        for (characters, from, to, base, segment) in segments {
            let to = to.min(from + remaining);
            let part = &characters[from..to];
            if part.is_empty() {
                continue;
            }
            for width in [2, 3] {
                ranges[segment * 2 + width - 2] =
                    (base + from, base + to.saturating_sub(width - 1).max(from));
            }
            let mut boundary = tail;
            let tail_len = boundary.len();
            boundary.extend(part.iter().take(2).map(|(character, _)| *character));
            for width in [2, 3] {
                for window in boundary.windows(width).take(tail_len) {
                    *probes += 1;
                    let gram = window.iter().collect::<String>();
                    if let Some(&id) = self.query_ids.get(&gram) {
                        self.local.add(id);
                        self.boundary.push(id);
                    }
                }
            }
            if part.len() >= 2 {
                tail = part[part.len() - 2..]
                    .iter()
                    .map(|(character, _)| *character)
                    .collect();
            } else {
                tail = boundary[boundary.len().saturating_sub(2)..].to_vec();
            }
            remaining -= part.len();
            if remaining == 0 {
                break;
            }
        }
        for (slot, (start, end)) in ranges.into_iter().enumerate() {
            let postings = if slot / 2 == 1 {
                &self.filtered_grams[slot % 2]
            } else {
                &self.token_grams[slot % 2]
            };
            self.local.update(slot, postings, start, end, probes);
        }
        self.local.overlap
    }
}

fn record_task_grams(
    characters: &[(char, usize)],
    base: usize,
    query: &BTreeMap<String, usize>,
    postings: &mut [Vec<(usize, usize)>; 2],
) {
    for width in [2, 3] {
        for (index, window) in characters.windows(width).enumerate() {
            let gram = window
                .iter()
                .map(|(character, _)| *character)
                .collect::<String>();
            if let Some(&id) = query.get(&gram) {
                postings[width - 2].push((base + index, id));
            }
        }
    }
}

pub(super) fn ngram_overlap_matches(query_len: usize, overlap: usize) -> bool {
    let minimum = if query_len <= 2 { 1 } else { 2 };
    query_len > 0 && overlap >= minimum && overlap as f64 / query_len as f64 >= 0.4
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
    (ngram_overlap_matches(query.len(), intersection) && union > 0)
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
