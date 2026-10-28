//! Fuzzy matching for the command palette and the settings search: a port of Textual's `FuzzySearch`, so hits come in
//! the order, and with the letters highlighted, that the Python app shows.
//!
//! Textual's scorer isn't a standard one. A query whose lowercased form is a substring of the candidate's matches its
//! first occurrence. Otherwise every increasing choice of positions for the query's letters is scored and the best
//! wins, the lexicographically first on a tie. A choice scores one point per letter plus one per letter at the start
//! of a word, boosted when the letters fall in fewer runs.

/// A query's match in a candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    /// How well the query matches: higher sorts first. Always positive.
    pub score: f64,
    /// The `char` indices of the candidate's matched letters, ascending.
    pub offsets: Vec<usize>,
}

impl FuzzyMatch {
    /// The offsets to draw highlighted in `candidate`: every matched letter that isn't whitespace.
    pub fn highlights(&self, candidate: &str) -> Vec<usize> {
        let chars: Vec<char> = candidate.chars().collect();
        self.offsets
            .iter()
            .copied()
            .filter(|&offset| chars.get(offset).is_some_and(|&c| !is_python_space(c)))
            .collect()
    }
}

/// Matches `query` against `candidate`, ignoring case: `None` unless the query's letters appear in the candidate in
/// order. An empty query is `None` too; callers list everything for it instead (Textual's would raise).
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let name: Vec<char> = candidate.to_lowercase().chars().collect();
    if query.is_empty() {
        return None;
    }
    let word_starts = word_starts(&name);

    if let Some(start) = name.windows(query.len()).position(|window| window == query) {
        let offsets: Vec<usize> = (start..start + query.len()).collect();
        let boost = if name == query { 2.0 } else { 1.5 };
        let score = score(&offsets, &word_starts) * boost;
        return Some(FuzzyMatch { score, offsets });
    }

    let positions = letter_positions(&query, &name)?;
    let mut best = None;
    best_offsets(
        &positions,
        &word_starts,
        &mut Vec::with_capacity(query.len()),
        &mut best,
    );
    best
}

/// The candidate positions for each of the query's letters, as Textual finds them: each letter's search starts one
/// past the first position of the previous letter's, and stops after a position at or past
/// `name.len() - letter_index - 1` (Textual's `last_index` cut). `None` when a letter has no position.
fn letter_positions(query: &[char], name: &[char]) -> Option<Vec<Vec<usize>>> {
    let mut all = Vec::with_capacity(query.len());
    let mut start = 0;
    for (letter_index, &letter) in query.iter().enumerate() {
        let last_index = name.len().saturating_sub(letter_index);
        let mut positions = Vec::new();
        let mut from = start;
        while let Some(found) = name.get(from..).and_then(|rest| rest.iter().position(|&c| c == letter)) {
            let location = from + found;
            positions.push(location);
            from = location + 1;
            if from >= last_index {
                break;
            }
        }
        start = positions.first()? + 1;
        all.push(positions);
    }
    Some(all)
}

/// Visits every increasing choice of one position per letter in lexicographic order, keeping the first that scores
/// highest in `best`.
fn best_offsets(
    positions: &[Vec<usize>],
    word_starts: &[bool],
    chosen: &mut Vec<usize>,
    best: &mut Option<FuzzyMatch>,
) {
    let Some((letter, rest)) = positions.split_first() else {
        let score = score(chosen, word_starts);
        if best.as_ref().is_none_or(|best| score > best.score) {
            *best = Some(FuzzyMatch {
                score,
                offsets: chosen.clone(),
            });
        }
        return;
    };
    for &offset in letter {
        if chosen.last().is_none_or(|&last| offset > last) {
            chosen.push(offset);
            best_offsets(rest, word_starts, chosen, best);
            chosen.pop();
        }
    }
}

/// Textual's `FuzzySearch.score` for non-empty `offsets`.
fn score(offsets: &[usize], word_starts: &[bool]) -> f64 {
    let count = offsets.len();
    let at_word_starts = offsets
        .iter()
        .filter(|&&offset| word_starts.get(offset).copied().unwrap_or(false))
        .count();
    let groups = 1 + offsets.windows(2).filter(|pair| pair[1] != pair[0] + 1).count();
    // Ratios of small integers, computed in the order Textual does so the floats agree to the bit. The counts are at
    // most the query's length, far below 2^52, so the casts are exact.
    #[allow(clippy::cast_precision_loss)]
    let normalized_groups = (count - (groups - 1)) as f64 / count as f64;
    #[allow(clippy::cast_precision_loss)]
    let base = (count + at_word_starts) as f64;
    base * (1.0 + normalized_groups * normalized_groups)
}

/// Whether each char of `name` starts a run of word characters (a `\w+` match).
fn word_starts(name: &[char]) -> Vec<bool> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    name.iter()
        .enumerate()
        .map(|(index, &c)| is_word(c) && (index == 0 || !is_word(name[index - 1])))
        .collect()
}

/// Python's `str.isspace`, which also counts the ASCII separators U+001C to U+001F.
fn is_python_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offsets(query: &str, candidate: &str) -> Vec<usize> {
        fuzzy_match(query, candidate).map(|m| m.offsets).unwrap_or_default()
    }

    fn assert_score(hit: &FuzzyMatch, expected: f64) {
        assert!(
            (hit.score - expected).abs() < 1e-9,
            "score {} isn't {expected}",
            hit.score
        );
    }

    #[test]
    fn letters_out_of_order_or_missing_are_no_match() {
        assert_eq!(fuzzy_match("zzz", "Toggle Issues"), None);
        assert_eq!(fuzzy_match("tiq", "Quit"), None);
        assert_eq!(fuzzy_match("tiuq", "Quit"), None);
    }

    #[test]
    fn empty_query_is_no_match() {
        assert_eq!(fuzzy_match("", "Quit"), None);
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(fuzzy_match("TOG", "toggle issues"), fuzzy_match("tog", "Toggle Issues"));
        // [0, 2, 7] and [0, 3, 7] tie; the first wins.
        assert_eq!(offsets("TgI", "Toggle Issues"), vec![0, 2, 7]);
    }

    #[test]
    fn exact_name_doubles_the_score() {
        // Four letters, one at a word start, one group: 5 × 2 × 2.0.
        let hit = fuzzy_match("Quit", "quit").unwrap();
        assert_eq!(hit.offsets, vec![0, 1, 2, 3]);
        assert_score(&hit, 5.0 * 2.0 * 2.0);
    }

    #[test]
    fn substring_scores_its_first_occurrence_times_one_and_a_half() {
        let hit = fuzzy_match("log", "Toggle Command Log").unwrap();
        // "log" first occurs at "Log", a word start: (3 + 1) × 2 × 1.5.
        assert_eq!(hit.offsets, vec![15, 16, 17]);
        assert_score(&hit, 4.0 * 2.0 * 1.5);
        let hit = fuzzy_match("g", "Toggle").unwrap();
        assert_eq!(hit.offsets, vec![2]);
        assert_score(&hit, 1.0 * 2.0 * 1.5);
    }

    #[test]
    fn subsequence_prefers_word_starts_and_fewer_groups() {
        let hit = fuzzy_match("tpr", "Toggle Pull Requests").unwrap();
        assert_eq!(hit.offsets, vec![0, 7, 12]);
        // Three letters, all word starts, three groups: 6 × (1 + (1/3)²).
        assert_score(&hit, 6.0 * (1.0 + (1.0 / 3.0) * (1.0 / 3.0)));
    }

    #[test]
    fn ties_go_to_the_lexicographically_first_offsets() {
        // [0, 2], [0, 6] and [4, 6] all score 3.75.
        let hit = fuzzy_match("ab", "a_b a_b").unwrap();
        assert_eq!(hit.offsets, vec![0, 2]);
        assert_score(&hit, 3.75);
    }

    #[test]
    fn the_last_index_cut_drops_later_positions() {
        // The `a` search (second letter) stops after index 2 = len - 1 - 1, so index 3 is never a candidate.
        assert_eq!(
            letter_positions(&['x', 'a'], &['x', 'b', 'a', 'a']),
            Some(vec![vec![0], vec![2]])
        );
        // Without the cut [7, 8, 10] would win, two groups ending on the word "b".
        assert_eq!(offsets("bbb", "Tab 22 bb b"), vec![2, 7, 8]);
    }

    #[test]
    fn whitespace_offsets_are_not_highlighted() {
        let hit = fuzzy_match("e i", "Toggle Issues").unwrap();
        assert_eq!(hit.offsets, vec![5, 6, 7]);
        assert_eq!(hit.highlights("Toggle Issues"), vec![5, 7]);
    }
}
