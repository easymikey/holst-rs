use std::borrow::Borrow;

use itertools::Itertools as _;

use crate::domain::{
    account::Workspace,
    limit::Limit,
    node::NodeKind,
    tree::name_index::build::{
        NameIndex,
        NodeIndex,
        canonical_uuid,
        normalized,
        offset,
        widen,
    },
};

/// Which nodes take part in a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeFilter {
    pub kind: Option<NodeKind>,
    pub in_trash: bool,
}

/// A found node and its score (higher is better).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeHit {
    pub node: NodeIndex,
    pub score: u32,
}

pub const EXACT: u32 = 1000;

pub const SUBSTRING: u32 = 400;

pub(crate) const ALL_WORDS: u32 = 200;

pub(crate) const WHOLE_WORD: u32 = 50;

impl NameIndex {
    /// The node with this id.
    #[must_use]
    pub fn find_id(&self, id: &str) -> Option<NodeIndex> {
        canonical_uuid(id)
            .map_or_else(
                || self.odd_node(id),
                |key| {
                    self.ids.iter().zip(0..).find_map(|(&candidate, node)| {
                        (candidate == key).then_some(node)
                    })
                },
            )
            .map(NodeIndex)
    }
    /// The folders above a node: `Folder/Subfolder`; empty for a root.
    #[must_use]
    pub fn folder_path(&self, node: NodeIndex) -> String {
        self.folder_names(node).join("/")
    }

    /// The names of the folders above a node, outermost first.
    #[must_use]
    pub fn folder_names(&self, node: NodeIndex) -> Vec<String> {
        // At most 32 levels: cycles in broken data end here.
        std::iter::successors(self.parent(node), |&up| self.parent(up))
            .take(32)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|up| self.name(up).to_owned())
            .collect()
    }
    /// How many live boards sit in `folder`, at any depth.
    #[must_use]
    pub fn boards_within(&self, folder: NodeIndex) -> usize {
        let inside = |node: NodeIndex| {
            std::iter::successors(self.parent(node), |&up| self.parent(up))
                .take(self.len())
                .any(|up| up == folder)
        };
        self.nodes()
            .filter(|&node| self.kind(node) == NodeKind::Board && !self.in_trash(node))
            .filter(|&node| inside(node))
            .count()
    }
    /// Best `limit` matches first, plus the total; one `memmem`
    /// pass.
    #[must_use]
    pub fn search(
        &self,
        query: &str,
        filter: NodeFilter,
        limit: Limit,
    ) -> (Vec<NodeHit>, usize) {
        let text = normalized(query);
        if text.is_empty() || self.is_empty() {
            (Vec::new(), 0)
        } else {
            self.search_normalized(&text, filter, limit)
        }
    }
    fn search_normalized(
        &self,
        query: &str,
        filter: NodeFilter,
        limit: Limit,
    ) -> (Vec<NodeHit>, usize) {
        let scored = overlapping(self.normalized_names.as_bytes(), query.as_bytes())
            .map(|position| self.score(position, query.len()));
        // A space in a normalized query means several words.
        let hits = match query.split_once(' ') {
            None => self.hits(best_per_node(scored), filter),
            Some(_) => self.hits(
                self.mark_all_words(self.dense_best(scored), query)
                    .into_iter()
                    .zip(0..)
                    .map(|(score, node)| (node, score)),
                filter,
            ),
        };
        let total = hits.len();
        (self.best_first(hits, limit), total)
    }
    pub(crate) fn dense_best(
        &self,
        scored: impl Iterator<Item = (u32, u32)>,
    ) -> Vec<u32> {
        scored.fold(vec![0u32; self.len()], |mut best, (node, score)| {
            best.get_mut(widen(node))
                .into_iter()
                .for_each(|slot| *slot = (*slot).max(score));
            best
        })
    }
    pub(crate) fn hits(
        &self,
        scored: impl Iterator<Item = (u32, u32)>,
        filter: NodeFilter,
    ) -> Vec<NodeHit> {
        scored
            .filter_map(|(node, score)| {
                let kind = *self.kinds.get(widen(node))?;
                let trash = *self.in_trash.get(widen(node))?;
                (score > 0
                    && trash == filter.in_trash
                    && filter.kind.is_none_or(|k| kind == k))
                .then(|| NodeHit {
                    node: NodeIndex(node),
                    score: score + u32::from(kind == NodeKind::Board),
                })
            })
            .collect()
    }
    fn mark_all_words(&self, mut best: Vec<u32>, query: &str) -> Vec<u32> {
        let word_count = query.split(' ').count();
        best.iter_mut()
            .zip(self.words_found(query))
            .filter(|(score, found)| **score == 0 && usize::from(*found) == word_count)
            .for_each(|(score, _)| *score = ALL_WORDS);
        best
    }
    fn words_found(&self, query: &str) -> Vec<u8> {
        let arena = self.normalized_names.as_bytes();
        let starts_word = |position: &usize| {
            position
                .checked_sub(1)
                .and_then(|prev| arena.get(prev))
                .is_none_or(|&b| matches!(b, b' ' | b'\n'))
        };
        query.split(' ').zip(1..=u8::MAX).fold(
            vec![0u8; self.len()],
            |mut seen, (word, found)| {
                overlapping(arena, word.as_bytes())
                    .filter(starts_word)
                    .for_each(|position| {
                        seen.get_mut(widen(self.node_at(position)))
                            .filter(|count| **count == found - 1)
                            .into_iter()
                            .for_each(|count| *count = found);
                    });
                seen
            },
        )
    }
    fn best_first(&self, mut hits: Vec<NodeHit>, limit: Limit) -> Vec<NodeHit> {
        let order = |a: &NodeHit, b: &NodeHit| {
            b.score
                .cmp(&a.score)
                .then_with(|| {
                    self.normalized_name(a.node.0)
                        .len()
                        .cmp(&self.normalized_name(b.node.0).len())
                })
                .then(a.node.cmp(&b.node))
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
    pub(crate) fn score(&self, position: usize, len: usize) -> (u32, u32) {
        let arena = self.normalized_names.as_bytes();
        let node = self.node_at(position);
        let end = position + len;
        let name_end =
            offset(&self.normalized_ends, widen(node)).unwrap_or(arena.len());
        let at = MatchStart::of(
            position == self.normalized_start(node),
            position.checked_sub(1).and_then(|prev| arena.get(prev)) == Some(&b' '),
        );
        // "1 15" in "1 15 plan" ranks above "1 150 plan".
        let whole_word = end == name_end || arena.get(end) == Some(&b' ');
        let score = match (at, end == name_end) {
            (MatchStart::NameStart, true) => EXACT,
            (place, _) => place.score() + WHOLE_WORD * u32::from(whole_word),
        };
        (node, score)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchStart {
    NameStart,
    WordStart,
    InsideWord,
}

impl MatchStart {
    pub(crate) fn of(name_start: bool, after_space: bool) -> Self {
        match (name_start, after_space) {
            (true, _) => Self::NameStart,
            (false, true) => Self::WordStart,
            (false, false) => Self::InsideWord,
        }
    }

    pub(crate) const fn score(self) -> u32 {
        match self {
            MatchStart::NameStart => 800,
            MatchStart::WordStart => 600,
            MatchStart::InsideWord => SUBSTRING,
        }
    }
}

pub(crate) fn best_per_node(
    scored: impl Iterator<Item = (u32, u32)>,
) -> std::vec::IntoIter<(u32, u32)> {
    scored
        .fold(Vec::new(), |mut best: Vec<(u32, u32)>, (node, score)| {
            match best.last_mut() {
                Some(last) if last.0 == node => last.1 = last.1.max(score),
                _ => best.push((node, score)),
            }
            best
        })
        .into_iter()
}

pub(crate) fn overlapping<'h>(
    haystack: &'h [u8],
    needle: &[u8],
) -> impl Iterator<Item = usize> + 'h {
    let finder = memchr::memmem::Finder::new(needle).into_owned();
    std::iter::successors(finder.find(haystack), move |&position| {
        let next = position + 1;
        haystack
            .get(next..)
            .and_then(|rest| finder.find(rest))
            .map(|found| next + found)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameGap {
    Leading,
    Word,
    Pending,
}

impl NameGap {
    pub(crate) const fn separated(self) -> Self {
        match self {
            NameGap::Word | NameGap::Pending => NameGap::Pending,
            NameGap::Leading => NameGap::Leading,
        }
    }
}

type SearchedTree<'a> = (&'a Workspace, &'a NameIndex, (Vec<NodeHit>, usize));

/// The live boards of the trees, by id with their names, in tree order.
#[must_use]
pub fn board_names<I: Borrow<NameIndex>>(
    trees: &[(Workspace, I)],
) -> Vec<(String, String)> {
    trees
        .iter()
        .map(|(_, index)| index.borrow())
        .flat_map(|index| {
            index
                .nodes()
                .filter(|&node| {
                    index.kind(node) == NodeKind::Board && !index.in_trash(node)
                })
                .map(|node| (index.id(node), index.name(node).to_owned()))
        })
        .collect()
}

/// A name hit: the workspace, its tree and the node found.
pub type WorkspaceNameHit<'a> = (&'a Workspace, &'a NameIndex, NodeHit);

/// The best `limit` name hits across the trees, and the total.
#[must_use]
pub fn name_hits<'a, I: Borrow<NameIndex>>(
    trees: &'a [(Workspace, I)],
    query: &str,
    filter: NodeFilter,
    limit: Limit,
) -> (Vec<WorkspaceNameHit<'a>>, usize) {
    let searched: Vec<SearchedTree<'a>> = trees
        .iter()
        .map(|(workspace, index)| {
            let index = index.borrow();
            (workspace, index, index.search(query, filter, limit))
        })
        .collect();
    let total = searched.iter().map(|(.., (_, count))| count).sum();
    // Stable: on a tie the earlier workspace and hit stay first.
    let hits: Vec<WorkspaceNameHit<'a>> = searched
        .into_iter()
        .flat_map(|(workspace, index, (found, _))| {
            found.into_iter().map(move |hit| (workspace, index, hit))
        })
        .sorted_by(|(_, index, hit), (_, other_index, other)| {
            other.score.cmp(&hit.score).then_with(|| {
                index
                    .name(hit.node)
                    .len()
                    .cmp(&other_index.name(other.node).len())
            })
        })
        .take(limit.count().unwrap_or(usize::MAX))
        .collect();
    (hits, total)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::{NodeFilter, name_hits};
    use crate::domain::{
        account::Workspace,
        limit::Limit,
        tree::name_index::build::NameIndex,
    };

    fn tree_of_boards(names: &[&str]) -> NameIndex {
        let channels: Vec<_> = names
            .iter()
            .zip(1..)
            .map(|(name, serial)| {
                json!({
                    "id": format!("00000001-0000-4000-8000-{serial:012x}"),
                    "type": "board",
                    "name": name,
                })
            })
            .collect();
        NameIndex::from_tree_json(&json!({ "channels": channels }).to_string())
            .unwrap_or_default()
    }

    /// `holst find plan --limit 0` once showed one hit: zero must keep all.
    #[rstest]
    #[case::zero_is_all(0, 3)]
    #[case::one(1, 1)]
    fn name_hits_keep_as_many_as_the_limit_says(
        #[case] asked: usize,
        #[case] kept: usize,
    ) {
        let trees = [
            (workspace("first"), tree_of_boards(&["plan a", "plan b"])),
            (workspace("second"), tree_of_boards(&["plan c", "other"])),
        ];
        let (hits, total) =
            name_hits(&trees, "plan", NodeFilter::default(), Limit::from(asked));
        assert_eq!((hits.len(), total), (kept, 3));
    }

    fn workspace(name: &str) -> Workspace {
        Workspace {
            id: name.into(),
            name: name.into(),
        }
    }
}
