//! Finding items by a reference: match rules as data, strongest first.

#[cfg(test)] mod tests;

use itertools::Itertools as _;

use crate::{
    domain::{
        account::Workspace,
        tree::name_index::{
            build::{normalize_into, normalized},
            query::{EXACT, SUBSTRING},
        },
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        lookup,
        lookup::LookupError,
    },
};

/// One way a reference can match an item, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchRule {
    ExactId,
    ExactName,
    IdPrefix,
    NamePart,
}

impl MatchRule {
    fn matches(self, id: &str, name: &str, reference: &Reference) -> bool {
        match self {
            MatchRule::ExactId => {
                !reference.id.is_empty() && id.eq_ignore_ascii_case(&reference.id)
            }
            MatchRule::ExactName => {
                !reference.name.is_empty() && name == reference.name
            }
            MatchRule::IdPrefix => {
                reference.id_prefix
                    && id
                        .get(..reference.id.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(&reference.id))
            }
            MatchRule::NamePart => {
                !reference.name.is_empty() && name.contains(reference.name.as_str())
            }
        }
    }

    const fn bit(self) -> u8 {
        match self {
            MatchRule::ExactId => 1,
            MatchRule::ExactName => 1 << 1,
            MatchRule::IdPrefix => 1 << 2,
            MatchRule::NamePart => 1 << 3,
        }
    }
}

/// A set of rules: those an item matches, or those a stronger match hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MatchRules(u8);

impl MatchRules {
    pub const NONE: Self = Self(0);

    /// The same rules and `rule`.
    #[must_use]
    pub const fn with(self, rule: MatchRule) -> Self {
        Self(self.0 | rule.bit())
    }

    /// Whether `rule` is among them.
    #[must_use]
    pub const fn has(self, rule: MatchRule) -> bool {
        self.0 & rule.bit() != 0
    }

    /// Whether there is no rule.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn within(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The rules a tree search scored `score` means: a whole name holds its
    /// part too.
    #[must_use]
    pub const fn of_score(score: u32) -> Self {
        match score {
            score if score >= EXACT => MatchRules::NONE
                .with(MatchRule::ExactName)
                .with(MatchRule::NamePart),
            score if score >= SUBSTRING => MatchRules::NONE.with(MatchRule::NamePart),
            _ => MatchRules::NONE,
        }
    }
}

/// Rules strongest first; a match hides weaker rules unless the pair is level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchOrder {
    pub rules: &'static [MatchRule],
    pub level: &'static [LevelPair],
}

/// Two rules of equal strength: neither hides the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelPair {
    pub first: MatchRule,
    pub second: MatchRule,
}

pub const WORKSPACE_ORDER: MatchOrder = MatchOrder {
    rules: &[
        MatchRule::ExactId,
        MatchRule::ExactName,
        MatchRule::NamePart,
    ],
    level: &[],
};

pub const NAME_ORDER: MatchOrder = MatchOrder {
    rules: &[MatchRule::ExactName, MatchRule::NamePart],
    level: &[],
};

pub const WHOLE_NAME_ORDER: MatchOrder = MatchOrder {
    rules: &[MatchRule::ExactName],
    level: &[],
};

pub const OBJECT_ORDER: MatchOrder = MatchOrder {
    rules: &[MatchRule::ExactId, MatchRule::IdPrefix],
    level: &[],
};

pub const FRAME_ORDER: MatchOrder = MatchOrder {
    rules: &[
        MatchRule::ExactId,
        MatchRule::ExactName,
        MatchRule::IdPrefix,
        MatchRule::NamePart,
    ],
    level: &[
        LevelPair {
            first: MatchRule::ExactName,
            second: MatchRule::IdPrefix,
        },
        LevelPair {
            first: MatchRule::IdPrefix,
            second: MatchRule::NamePart,
        },
    ],
};

impl MatchOrder {
    fn all(&self) -> MatchRules {
        self.rules
            .iter()
            .fold(MatchRules::NONE, |rules, &rule| rules.with(rule))
    }

    fn is_level(&self, one: MatchRule, other: MatchRule) -> bool {
        self.level.iter().any(|&pair| {
            pair == LevelPair {
                first: one,
                second: other,
            } || pair
                == LevelPair {
                    first: other,
                    second: one,
                }
        })
    }

    fn rules_of(&self, id: &str, name: &str, reference: &Reference) -> MatchRules {
        self.rules
            .iter()
            .filter(|rule| rule.matches(id, name, reference))
            .fold(MatchRules::NONE, |rules, &rule| rules.with(rule))
    }

    fn hidden_by(&self, present: MatchRules) -> MatchRules {
        self.rules
            .iter()
            .enumerate()
            .filter(|&(_, &rule)| present.has(rule))
            .flat_map(|(position, &stronger)| {
                self.rules
                    .iter()
                    .skip(position + 1)
                    .filter(move |&&weaker| !self.is_level(stronger, weaker))
            })
            .fold(MatchRules::NONE, |hidden, &rule| hidden.with(rule))
    }

    /// Returns, in order, the items whose rules a stronger match in their
    /// group does not hide; rules outside this order are ignored.
    pub fn strongest<T, G: Copy + PartialEq>(
        &self,
        matched: impl IntoIterator<Item = (T, G, MatchRules)>,
    ) -> Vec<T> {
        let all = self.all();
        let matched: Vec<(T, G, MatchRules)> = matched
            .into_iter()
            .map(|(item, group, rules)| (item, group, rules.within(all)))
            .filter(|(_, _, rules)| !rules.is_empty())
            .collect();
        let present = matched.iter().fold(
            Vec::<(G, MatchRules)>::new(),
            |mut present, &(_, group, rules)| {
                match present.iter_mut().find(|(known, _)| *known == group) {
                    Some((_, seen)) => *seen = seen.union(rules),
                    None => present.push((group, rules)),
                }
                present
            },
        );
        let hidden: Vec<(G, MatchRules)> = present
            .into_iter()
            .map(|(group, rules)| (group, self.hidden_by(rules)))
            .collect();
        let hidden_in = |group: G| {
            hidden
                .iter()
                .find(|(known, _)| *known == group)
                .map_or(MatchRules::NONE, |&(_, rules)| rules)
        };
        matched
            .into_iter()
            .filter(|&(_, group, rules)| !rules.without(hidden_in(group)).is_empty())
            .map(|(item, ..)| item)
            .collect()
    }
}

/// What a reference names, read once: an id part and a normalized name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    id: String,
    name: String,
    id_prefix: bool,
}

impl Reference {
    /// A board object: an id, a link with `objectId=`, an id start (also
    /// with the `@` that `o ls` prints before it) or a title.
    #[must_use]
    pub fn object(raw: &str) -> Self {
        let trimmed = raw.trim();
        let bare = trimmed
            .strip_prefix('@')
            .filter(|rest| is_id_prefix(rest))
            .unwrap_or(trimmed);
        let id = crate::domain::id::object_in(raw)
            .map_or_else(|| bare.to_owned(), String::from);
        Self {
            id_prefix: is_id_prefix(&id),
            id,
            name: normalized(raw),
        }
    }

    /// A workspace: an id or link, else a name.
    #[must_use]
    pub fn workspace(raw: &str) -> Self {
        let id = crate::domain::id::workspace_in(raw).map(String::from);
        Self {
            id: id.as_deref().unwrap_or(raw).to_owned(),
            name: normalized(raw),
            id_prefix: false,
        }
    }

    fn is_empty(&self) -> bool {
        self.id.is_empty() && self.name.is_empty()
    }

    /// Whether it holds an id start, so a miss is a missing object.
    #[must_use]
    pub const fn is_id_prefix(&self) -> bool {
        self.id_prefix
    }
}

/// Returns, in input order, the items `order` keeps for `reference`; each
/// name is normalized once into one reused buffer.
pub fn find_matches<T>(
    items: impl IntoIterator<Item = T>,
    identify: impl Fn(&T) -> (&str, &str),
    reference: &Reference,
    order: &MatchOrder,
) -> Vec<T> {
    // imperative: one buffer reused for every name avoids an allocation per item.
    let mut name = String::new();
    // An emoji-only name normalizes to "", which names nothing: no scan.
    if reference.is_empty() {
        Vec::new()
    } else {
        order.strongest(items.into_iter().map(|item| {
            let (id, raw) = identify(&item);
            name.clear();
            normalize_into(raw, &mut name);
            let rules = order.rules_of(id, &name, reference);
            (item, (), rules)
        }))
    }
}

/// What a lookup by reference found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupMatch<'a, T> {
    NotFound,
    One(&'a T),
    Several(Vec<&'a T>),
}

impl<'a, T> From<Vec<&'a T>> for LookupMatch<'a, T> {
    fn from(matched: Vec<&'a T>) -> Self {
        match matched.as_slice() {
            [] => LookupMatch::NotFound,
            &[one] => LookupMatch::One(one),
            _ => LookupMatch::Several(matched),
        }
    }
}

/// Returns the items matched by the strongest rules of `order` that match any.
///
/// ```
/// use holst_core::domain::{
///     account::Workspace,
///     lookup::rules::{LookupMatch, Reference, WORKSPACE_ORDER, find_workspace},
/// };
///
/// let named = |id: &str, name: &str| Workspace {
///     id: id.into(),
///     name: name.into(),
/// };
/// let all = [named("w1", "Plans"), named("w2", "Plans Q4")];
/// let found = |reference| {
///     find_workspace(&all, &Reference::workspace(reference), &WORKSPACE_ORDER)
/// };
/// assert_eq!(found("plans"), LookupMatch::One(&all[0]));
/// assert_eq!(found("W2"), LookupMatch::One(&all[1]));
/// assert!(matches!(found("plan"), LookupMatch::Several(_)));
/// assert_eq!(found("retro"), LookupMatch::NotFound);
/// ```
#[must_use]
pub fn find_workspace<'a>(
    items: &'a [Workspace],
    reference: &Reference,
    order: &MatchOrder,
) -> LookupMatch<'a, Workspace> {
    LookupMatch::from(find_matches(
        items,
        |item| (item.id.as_str(), item.name.as_str()),
        reference,
        order,
    ))
}

const MIN_ID_PREFIX: usize = 8;

/// Whether `text` is an MCP alias: `@` and a decimal number shorter than an id
/// prefix (`@3`). Eight or more characters after the `@` are an id start.
#[must_use]
pub fn is_alias(text: &str) -> bool {
    text.trim().strip_prefix('@').is_some_and(|n| {
        !n.is_empty()
            && n.len() < MIN_ID_PREFIX
            && n.bytes().all(|b| b.is_ascii_digit())
    })
}

/// The id `reference` names: an id, a link with `objectId=`, or a long hex
/// prefix.
///
/// # Errors
/// [`crate::error::board_change::BoardChangeError::NoObject`], or a
/// [`crate::error::lookup::LookupError`] of an object.
pub fn find_id<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    reference: &str,
) -> Result<String> {
    let wanted = Reference::object(reference);
    let matching: Vec<&str> = find_matches(ids, |id| (*id, ""), &wanted, &OBJECT_ORDER)
        .into_iter()
        .sorted_unstable()
        .collect();
    match matching.as_slice() {
        [single] => Ok((*single).to_owned()),
        [] if !wanted.is_id_prefix() => {
            Err(Error::BoardChange(BoardChangeError::NoObject {
                reference: reference.into(),
            }))
        }
        [] => Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Object,
            reference: reference.into(),
            miss: lookup::LookupMiss::not_found(),
        })),
        many => Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Object,
            reference: reference.into(),
            miss: lookup::LookupMiss::ambiguous(lookup::Candidates::capped(
                many.iter().map(|id| lookup::Candidate {
                    name: (*id).to_owned(),
                    ..lookup::Candidate::default()
                }),
                5,
            )),
        })),
    }
}

fn is_id_prefix(text: &str) -> bool {
    let head_is_hex = text
        .get(..MIN_ID_PREFIX)
        .is_some_and(|head| head.chars().all(|c| c.is_ascii_hexdigit()));
    head_is_hex && text.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}
