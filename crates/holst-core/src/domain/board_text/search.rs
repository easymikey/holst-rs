//! Text search across many boards: one normalized arena, `memmem`, a typo.

#[cfg(test)] mod tests;

use std::{cmp::Ordering, ops::Range, sync::Arc};

use itertools::Itertools;

use crate::domain::{
    board::ObjectKind,
    board_text::index::BoardIndex,
    limit::Limit,
    tree::name_index::{
        build::{normalize_into, normalized, saturating_u32, widen},
        query::{ALL_WORDS, EXACT, MatchStart, WHOLE_WORD, overlapping},
    },
};

pub const FUZZY_PENALTY: u32 = 300;

pub const FUZZY_MIN_CHARS: usize = 4;

const CHARS_BEFORE: usize = 30;

const EXCERPT_CHARS: usize = 100;

const ELLIPSIS: &str = "…";

/// Which objects take part in a content search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectFilter {
    pub board: Option<String>,
    pub kind: Option<ObjectKind>,
}

/// A piece of an object's text around a match, on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub text: String,
    pub range: Range<usize>,
}

impl Excerpt {
    /// The text before the match, the match, and the text after it.
    #[must_use]
    pub fn parts(&self) -> (&str, &str, &str) {
        let text = self.text.as_str();
        (
            text.get(..self.range.start).unwrap_or_default(),
            text.get(self.range.clone()).unwrap_or_default(),
            text.get(self.range.end..).unwrap_or_default(),
        )
    }
}

/// An object whose text matched, best first in a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextHit {
    pub board: String,
    pub object: String,
    pub kind: ObjectKind,
    pub excerpt: Excerpt,
    pub score: u32,
    pub fuzzy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CorpusEntry {
    board: u32,
    object: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScoredMatch {
    entry: u32,
    score: u32,
    at: u32,
    len: u32,
    fuzzy: bool,
}

impl ScoredMatch {
    fn better(self, other: Self) -> Self {
        match other.score.cmp(&self.score) {
            Ordering::Greater => other,
            Ordering::Less | Ordering::Equal => self,
        }
    }
}

/// The normalized texts of many boards' objects, searchable together.
#[derive(Debug)]
pub struct TextCorpus<'a> {
    boards: Vec<&'a BoardIndex>,
    normalized: String,
    ends: Vec<u32>,
    entries: Vec<CorpusEntry>,
}

impl<'a> From<&'a [Arc<BoardIndex>]> for TextCorpus<'a> {
    fn from(boards: &'a [Arc<BoardIndex>]) -> Self {
        Self::of(boards.iter().map(AsRef::as_ref))
    }
}

impl<'a> TextCorpus<'a> {
    /// The corpus of every object with text in `boards`, whatever source gave
    /// the indexes.
    #[must_use]
    pub fn of(boards: impl IntoIterator<Item = &'a BoardIndex>) -> Self {
        let boards: Vec<&'a BoardIndex> = boards.into_iter().collect();
        let capacity = boards
            .iter()
            .flat_map(|board| (0..board.len()).map(|at| board.text(at).len() + 1))
            .sum();
        let empty = Self {
            boards: boards.clone(),
            normalized: String::with_capacity(capacity),
            ends: Vec::new(),
            entries: Vec::new(),
        };
        boards
            .iter()
            .zip(0u32..)
            .fold(empty, |corpus, (board, number)| {
                (0..board.len()).fold(corpus, |mut corpus, at| {
                    corpus.push(
                        board.text(at),
                        CorpusEntry {
                            board: number,
                            object: saturating_u32(at),
                        },
                    );
                    corpus
                })
            })
            .shrunk()
    }
}

impl TextCorpus<'_> {
    fn shrunk(mut self) -> Self {
        self.normalized.shrink_to_fit();
        self
    }

    fn push(&mut self, text: &str, entry: CorpusEntry) {
        let start = self.normalized.len();
        normalize_into(text, &mut self.normalized);
        let added = self.normalized.len() > start;
        self.ends
            .extend(added.then(|| saturating_u32(self.normalized.len())));
        self.normalized.extend(added.then_some('\n'));
        self.entries.extend(added.then_some(entry));
    }

    /// Whether no object has text.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The best `limit` hits (`0` — all) and how many there are in total.
    ///
    /// ```
    /// use holst_core::domain::{
    ///     board::{Object, ObjectKind},
    ///     board_text::{
    ///         index::BoardIndex,
    ///         search::{ObjectFilter, TextCorpus},
    ///     },
    ///     limit::Limit,
    /// };
    ///
    /// let sticker = |id: &str, text: &str| Object {
    ///     id: id.into(),
    ///     kind: ObjectKind::Sticker,
    ///     parent: None,
    ///     x: 0.0,
    ///     y: 0.0,
    ///     width: None,
    ///     height: None,
    ///     z_index: 0.0,
    ///     text: text.into(),
    ///     color: None,
    ///     look: None,
    ///     ends: None,
    ///     author: None,
    ///     locked: false,
    ///     rotation: 0.0,
    ///     file: None,
    ///     fields: serde_json::Value::Null,
    /// };
    /// let boards = [BoardIndex::from_objects(
    ///     &[sticker("a", "Ёжик в тумане"), sticker("b", "Ежевика")],
    ///     "board",
    ///     Vec::new(),
    /// )];
    /// let corpus = TextCorpus::of(boards.iter());
    /// let (hits, total) =
    ///     corpus.search("ежик", &ObjectFilter::default(), Limit::from(10));
    /// assert_eq!((hits[0].object.as_str(), total), ("a", 1));
    /// assert_eq!(hits[0].excerpt.parts(), ("", "Ёжик", " в тумане"));
    /// ```
    #[must_use]
    pub fn search(
        &self,
        query: &str,
        filter: &ObjectFilter,
        limit: Limit,
    ) -> (Vec<TextHit>, usize) {
        let text = normalized(query);
        if text.is_empty() || self.is_empty() {
            (Vec::new(), 0)
        } else {
            let hits = self.matches(&text, filter, limit);
            let total = hits.len();
            let shown = self.best_first(hits, limit);
            (
                shown.into_iter().filter_map(|hit| self.hit(hit)).collect(),
                total,
            )
        }
    }

    fn matches(
        &self,
        query: &str,
        filter: &ObjectFilter,
        limit: Limit,
    ) -> Vec<ScoredMatch> {
        let words: Vec<&str> = query.split(' ').collect();
        let phrase = best_per_entry(
            overlapping(self.normalized.as_bytes(), query.as_bytes())
                .map(|at| self.score(at, query.len())),
        );
        let all_words = match words.as_slice() {
            [] | [_] => Vec::new(),
            words => self.all_words(words, &phrase),
        };
        let exact: Vec<ScoredMatch> = phrase
            .into_iter()
            .chain(all_words)
            .filter(|hit| self.passes(hit, filter))
            .collect();
        let char_count = query.chars().count();
        let typo_allowed = words.len() == 1
            && char_count >= FUZZY_MIN_CHARS
            && limit.count().is_none_or(|count| exact.len() < count);
        let fuzzy = typo_allowed.then(|| self.fuzzy(query, &exact));
        exact
            .into_iter()
            .chain(
                fuzzy
                    .into_iter()
                    .flatten()
                    .filter(|hit| self.passes(hit, filter)),
            )
            .collect()
    }

    fn score(&self, at: usize, len: usize) -> ScoredMatch {
        let entry = self.entry_at(at);
        let (start, end) = self.span(entry);
        let arena = self.normalized.as_bytes();
        let stop = at + len;
        let place = MatchStart::of(at == start, starts_word(arena, at));
        let whole_word = stop == end || arena.get(stop) == Some(&b' ');
        let score = match (place, stop == end) {
            (MatchStart::NameStart, true) => EXACT,
            (place, _) => place.score() + WHOLE_WORD * u32::from(whole_word),
        };
        ScoredMatch {
            entry,
            score,
            at: saturating_u32(at),
            len: saturating_u32(len),
            fuzzy: false,
        }
    }

    fn all_words(&self, words: &[&str], phrase: &[ScoredMatch]) -> Vec<ScoredMatch> {
        let count = words.len().min(usize::from(u8::MAX));
        self.words_found(words)
            .into_iter()
            .zip(0u32..)
            .filter(|(found, _)| usize::from(*found) == count)
            .map(|(_, entry)| entry)
            .filter(|entry| {
                phrase.binary_search_by_key(entry, |hit| hit.entry).is_err()
            })
            .filter_map(|entry| self.all_words_at(entry, words))
            .collect()
    }

    fn words_found(&self, words: &[&str]) -> Vec<u8> {
        let arena = self.normalized.as_bytes();
        words.iter().zip(1..=u8::MAX).fold(
            vec![0u8; self.entries.len()],
            |mut seen, (word, found)| {
                overlapping(arena, word.as_bytes())
                    .filter(|&at| starts_word(arena, at))
                    .for_each(|at| {
                        seen.get_mut(widen(self.entry_at(at)))
                            .filter(|count| **count == found - 1)
                            .into_iter()
                            .for_each(|count| *count = found);
                    });
                seen
            },
        )
    }

    fn all_words_at(&self, entry: u32, words: &[&str]) -> Option<ScoredMatch> {
        let (start, end) = self.span(entry);
        let text = self.normalized.get(start..end)?.as_bytes();
        let first = words.first()?;
        let at = word_starts(text, first.as_bytes()).next()?;
        let whole = words.iter().all(|word| {
            word_starts(text, word.as_bytes()).any(|word_start| {
                text.get(word_start + word.len())
                    .is_none_or(|&next| next == b' ')
            })
        });
        Some(ScoredMatch {
            entry,
            score: ALL_WORDS + WHOLE_WORD * u32::from(whole),
            at: saturating_u32(start + at),
            len: saturating_u32(first.len()),
            fuzzy: false,
        })
    }

    fn fuzzy(&self, query: &str, exact: &[ScoredMatch]) -> Vec<ScoredMatch> {
        let chars: Vec<char> = query.chars().collect();
        let mid = chars.len() / 2;
        let piece = |part: &[char]| part.iter().collect::<String>();
        let swapped: Vec<char> = chars
            .iter()
            .enumerate()
            .map(|(at, &ch)| match at.cmp(&mid) {
                Ordering::Equal => chars.get(mid - 1).copied().unwrap_or(ch),
                Ordering::Less if at + 1 == mid => {
                    chars.get(mid).copied().unwrap_or(ch)
                }
                Ordering::Less | Ordering::Greater => ch,
            })
            .collect();
        // An edit spares a half; a swap across the middle is sought whole.
        let pieces = [
            piece(chars.get(..mid).unwrap_or_default()),
            piece(chars.get(mid..).unwrap_or_default()),
            piece(&swapped),
        ];
        let arena = self.normalized.as_bytes();
        let starts: Vec<usize> = pieces
            .iter()
            .flat_map(|candidate| {
                overlapping(arena, candidate.as_bytes()).map(|at| word_start(arena, at))
            })
            .sorted_unstable()
            .dedup()
            .collect();
        let exact_entries: Vec<u32> = exact
            .iter()
            .map(|hit| hit.entry)
            .sorted_unstable()
            .collect();
        best_per_entry(
            starts
                .into_iter()
                .filter_map(|start| self.fuzzy_at(start, &chars))
                .filter(|hit| exact_entries.binary_search(&hit.entry).is_err()),
        )
    }

    fn fuzzy_at(&self, start: usize, query: &[char]) -> Option<ScoredMatch> {
        let arena = self.normalized.as_bytes();
        let rest = self.normalized.get(start..)?;
        let word_len = rest.find([' ', '\n']).unwrap_or(rest.len());
        let word: Vec<char> = rest.get(..word_len)?.chars().collect();
        let length = query.len();
        let whole = word.len() <= length + 1 && within_one(query, &word);
        let taken = whole.then_some(word.len()).or_else(|| {
            [length, length - 1, length + 1]
                .into_iter()
                .filter(|&count| count < word.len())
                .find(|&count| {
                    word.get(..count)
                        .is_some_and(|prefix| within_one(query, prefix))
                })
        })?;
        let len = rest
            .char_indices()
            .nth(taken)
            .map_or(rest.len(), |(byte, _)| byte);
        let entry = self.entry_at(start);
        let (entry_start, entry_end) = self.span(entry);
        let place = MatchStart::of(start == entry_start, starts_word(arena, start));
        let exact_text = whole && start == entry_start && start + len == entry_end;
        let score = match (exact_text, place) {
            (true, _) => EXACT,
            (false, place) => place.score() + WHOLE_WORD * u32::from(whole),
        };
        Some(ScoredMatch {
            entry,
            score: score.saturating_sub(FUZZY_PENALTY),
            at: saturating_u32(start),
            len: saturating_u32(len),
            fuzzy: true,
        })
    }

    fn passes(&self, hit: &ScoredMatch, filter: &ObjectFilter) -> bool {
        self.entries
            .get(widen(hit.entry))
            .and_then(|entry| {
                Some((self.boards.get(widen(entry.board))?, widen(entry.object)))
            })
            .is_some_and(|(board, object)| {
                filter.board.as_deref().is_none_or(|id| board.board() == id)
                    && filter
                        .kind
                        .as_ref()
                        .is_none_or(|kind| board.kind_name(object) == kind.as_str())
            })
    }

    fn best_first(&self, mut hits: Vec<ScoredMatch>, limit: Limit) -> Vec<ScoredMatch> {
        let length = |hit: &ScoredMatch| {
            let (start, end) = self.span(hit.entry);
            end - start
        };
        let order = |a: &ScoredMatch, b: &ScoredMatch| {
            b.score
                .cmp(&a.score)
                .then_with(|| length(a).cmp(&length(b)))
                .then(a.entry.cmp(&b.entry))
        };
        limit
            .count()
            .filter(|&limit| hits.len() > limit)
            .into_iter()
            .for_each(|limit| {
                hits.select_nth_unstable_by(limit - 1, order);
                hits.truncate(limit);
            });
        hits.sort_unstable_by(order);
        hits
    }

    fn hit(&self, hit: ScoredMatch) -> Option<TextHit> {
        let entry = self.entries.get(widen(hit.entry))?;
        let board = self.boards.get(widen(entry.board))?;
        let object = widen(entry.object);
        let (start, _) = self.span(hit.entry);
        let from = widen(hit.at).saturating_sub(start);
        let text = board.text(object);
        Some(TextHit {
            board: board.board().to_owned(),
            object: board.id(object).into_owned(),
            kind: ObjectKind::of(&board.kind_name(object)),
            excerpt: excerpt(text, source_range(text, from, from + widen(hit.len))),
            score: hit.score,
            fuzzy: hit.fuzzy,
        })
    }

    fn entry_at(&self, at: usize) -> u32 {
        saturating_u32(self.ends.partition_point(|&end| widen(end) <= at))
    }

    fn span(&self, entry: u32) -> (usize, usize) {
        let entry = widen(entry);
        let start = entry
            .checked_sub(1)
            .and_then(|before| self.ends.get(before))
            .map_or(0, |&end| widen(end) + 1);
        let end = self.ends.get(entry).map_or(start, |&end| widen(end));
        (start, end)
    }
}

fn best_per_entry(scored: impl Iterator<Item = ScoredMatch>) -> Vec<ScoredMatch> {
    scored.fold(Vec::new(), |mut best: Vec<ScoredMatch>, next| {
        match best.last_mut() {
            Some(last) if last.entry == next.entry => *last = last.better(next),
            _ => best.push(next),
        }
        best
    })
}

fn starts_word(arena: &[u8], at: usize) -> bool {
    at.checked_sub(1)
        .and_then(|before| arena.get(before))
        .is_none_or(|&byte| matches!(byte, b' ' | b'\n'))
}

fn word_start(arena: &[u8], at: usize) -> usize {
    arena
        .get(..at)
        .and_then(|before| {
            before
                .iter()
                .rposition(|&byte| matches!(byte, b' ' | b'\n'))
        })
        .map_or(0, |space| space + 1)
}

fn word_starts<'t>(text: &'t [u8], word: &[u8]) -> impl Iterator<Item = usize> + 't {
    overlapping(text, word).filter(move |&at| starts_word(text, at))
}

fn within_one(a: &[char], b: &[char]) -> bool {
    let first = a.iter().zip(b).position(|(x, y)| x != y);
    let (short, long) = match a.len().cmp(&b.len()) {
        Ordering::Greater => (b, a),
        Ordering::Less | Ordering::Equal => (a, b),
    };
    match (long.len() - short.len(), first) {
        (0, None) => true,
        (0, Some(at)) => {
            a.get(at + 1..) == b.get(at + 1..)
                || (a.get(at) == b.get(at + 1)
                    && a.get(at + 1) == b.get(at)
                    && a.get(at + 2..) == b.get(at + 2..))
        }
        (1, first) => {
            let at = first.unwrap_or(short.len());
            short.get(at..) == long.get(at + 1..)
        }
        _ => false,
    }
}

fn source_range(text: &str, from: usize, to: usize) -> Range<usize> {
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(at, _)| at)
        .chain([text.len()])
        .collect();
    let normal_len = |end: usize| normalized(text.get(..end).unwrap_or_default()).len();
    // The first prefix reaching past `from` ends in the match's first char.
    let reaching = bounds.partition_point(|&end| normal_len(end) <= from);
    let start = reaching
        .checked_sub(1)
        .and_then(|before| bounds.get(before))
        .copied()
        .unwrap_or(0);
    let covering = bounds.partition_point(|&end| normal_len(end) < to);
    let end = bounds.get(covering).copied().unwrap_or(text.len());
    start..end.max(start)
}

fn excerpt(text: &str, found: Range<usize>) -> Excerpt {
    let before = text.get(..found.start).unwrap_or_default();
    let from = before
        .char_indices()
        .rev()
        .nth(CHARS_BEFORE - 1)
        .map_or(0, |(at, _)| at);
    let shown_before = text
        .get(from..found.start)
        .map_or(0, |part| part.chars().count());
    let matched = text
        .get(found.clone())
        .map_or(0, |part| part.chars().count());
    let after_count = EXCERPT_CHARS.saturating_sub(shown_before + matched);
    let after = text.get(found.end..).unwrap_or_default();
    let to = found.end
        + after
            .char_indices()
            .nth(after_count)
            .map_or(after.len(), |(at, _)| at);
    let lead = match from {
        0 => "",
        _ => ELLIPSIS,
    };
    let tail = match to.cmp(&text.len()) {
        Ordering::Less => ELLIPSIS,
        Ordering::Equal | Ordering::Greater => "",
    };
    let window: String = text
        .get(from..to)
        .unwrap_or_default()
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ' ',
            ch => ch,
        })
        .collect();
    let shift = |at: usize| at - from + lead.len();
    Excerpt {
        text: format!("{lead}{window}{tail}"),
        range: shift(found.start)..shift(found.end),
    }
}
