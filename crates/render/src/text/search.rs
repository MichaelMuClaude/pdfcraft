//! Literal search in normalized page text, retaining the original glyph owners.
use super::{PageText, fold_case};
use std::ops::Range;

pub(super) fn find(page: &PageText, query: &str, sensitive: bool, whole_words: bool, limit: usize) -> Vec<Range<usize>> {
    if limit == 0 {
        return Vec::new();
    }
    let fold = |s: &str| if sensitive { s.to_owned() } else { fold_case(s) };
    let needle: Vec<char> = fold(query)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(|c| {
            let (letters, n) = unligature(c);
            letters.into_iter().take(n)
        })
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut chars: Vec<(char, usize)> = Vec::new();
    for (i, glyph) in page.glyphs.iter().enumerate() {
        let line_break = i.checked_sub(1).and_then(|previous| page.line_of.get(previous)).zip(page.line_of.get(i)).is_some_and(|(a, b)| a != b);
        let gap = page.space_before.get(i).copied().unwrap_or(false) || line_break;
        if i > 0 && gap && chars.last().is_some_and(|c| c.0 != ' ') {
            chars.push((' ', i));
        }
        for raw in fold(&glyph.text).chars() {
            let (letters, n) = unligature(raw);
            for c in letters.into_iter().take(n) {
                let c = if c.is_whitespace() { ' ' } else { c };
                if c != ' ' || chars.last().is_none_or(|last| last.0 != ' ') {
                    chars.push((c, i));
                }
            }
        }
    }
    if needle.len() > chars.len() {
        return Vec::new();
    }
    // KMP reuses a matched prefix instead of comparing a long query again at every position.
    // The table is bounded by the page's already materialized character count.
    let mut prefix = vec![0; needle.len()];
    let mut matched = 0;
    for (i, &c) in needle.iter().enumerate().skip(1) {
        matched = advance(&needle, &prefix, matched, c);
        if let Some(slot) = prefix.get_mut(i) {
            *slot = matched;
        }
    }
    matched = 0;
    let word = |i: Option<usize>| i.and_then(|i| chars.get(i)).is_some_and(|c| c.0.is_alphanumeric());
    let mut hits = Vec::new();
    for (i, &(c, owner)) in chars.iter().enumerate() {
        matched = advance(&needle, &prefix, matched, c);
        if matched != needle.len() {
            continue;
        }
        let end = i.saturating_add(1);
        let start = end.saturating_sub(needle.len());
        let bounded = !whole_words || (!word(start.checked_sub(1)) && !word(Some(end)));
        if bounded {
            if let Some((_, first)) = chars.get(start)
                && let Some(last) = owner.checked_add(1)
            {
                hits.push(*first..last);
                if hits.len() == limit {
                    break;
                }
            }
            // Only accepted hits consume their full span: literal search is non-overlapping.
            matched = 0;
        } else {
            // A word-boundary rejection must still allow an overlapping later candidate.
            matched = prefix.last().copied().unwrap_or_default();
        }
    }
    hits
}

/// `c`'s own letters, for the eight Latin ligatures a font's glyph name or ToUnicode CMap may map
/// to a single precomposed scalar instead of the sequence ISO 32000-2 §9.10.3 gives as its own
/// ToUnicode example (`U+0066 U+0069` for "fi"): the Adobe Glyph List does the same for a glyph
/// literally named `fi` (#4469). A search must read the two forms as the same letters whichever
/// one the page -- or the query -- happens to use. Anything else is just `c`, alone.
fn unligature(c: char) -> ([char; 3], usize) {
    match c {
        '\u{fb00}' => (['f', 'f', ' '], 2),
        '\u{fb01}' => (['f', 'i', ' '], 2),
        '\u{fb02}' => (['f', 'l', ' '], 2),
        '\u{fb03}' => (['f', 'f', 'i'], 3),
        '\u{fb04}' => (['f', 'f', 'l'], 3),
        '\u{fb05}' | '\u{fb06}' => (['s', 't', ' '], 2),
        _ => ([c, ' ', ' '], 1),
    }
}

/// Extend a prefix, falling back through strictly shorter proper prefixes on a mismatch.
fn advance(needle: &[char], prefix: &[usize], mut matched: usize, c: char) -> usize {
    while matched > 0 && needle.get(matched) != Some(&c) {
        matched = prefix.get(matched - 1).copied().unwrap_or_default();
    }
    if needle.get(matched) == Some(&c) { matched.saturating_add(1) } else { 0 }
}
