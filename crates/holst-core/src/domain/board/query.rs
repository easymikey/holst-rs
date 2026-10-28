//! Which board objects to list: a kind, a text and a page; no I/O.

use crate::{
    domain::{
        board::{Object, ObjectKind},
        limit::Limit,
        tree::name_index::build::normalized,
    },
    error::Result,
};

/// Which objects to list: of a kind, with a text, a page of the matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectQuery<'a> {
    pub kind: Option<ObjectKind>,
    pub text: Option<&'a str>,
    pub offset: usize,
    pub limit: Limit,
}

/// One page of the objects that match a query.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectPage<'a> {
    pub matched: usize,
    pub from: usize,
    pub shown: Vec<&'a Object>,
}

impl ObjectQuery<'_> {
    /// The page of `objects` the query asks for, in their order, among the
    /// `objects` that `keep` accepts; the kind is still checked against all of
    /// `objects`.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotObjectKind`] for a kind that has
    /// no variant and that none of `objects` has.
    pub fn page_where<'o>(
        &self,
        objects: &[&'o Object],
        keep: impl Fn(&Object) -> bool,
    ) -> Result<ObjectPage<'o>> {
        self.kind.iter().try_for_each(|kind| {
            kind.check_among(objects.iter().map(|object| object.kind.clone()))
        })?;
        let needle = self.text.map(normalized);
        let matched: Vec<&Object> = objects
            .iter()
            .copied()
            .filter(|object| self.kind.as_ref().is_none_or(|kind| object.kind == *kind))
            .filter(|object| keep(object))
            .filter(|object| {
                needle
                    .as_deref()
                    .is_none_or(|needle| normalized(&object.text).contains(needle))
            })
            .collect();
        let from = self.offset.min(matched.len());
        let shown = matched
            .iter()
            .copied()
            .skip(from)
            .take(self.limit.count().unwrap_or(usize::MAX))
            .collect();
        Ok(ObjectPage {
            matched: matched.len(),
            from,
            shown,
        })
    }
}

impl ObjectPage<'_> {
    /// One past the last match shown.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.from.saturating_add(self.shown.len())
    }

    /// The offset of the next page, when matches are left.
    #[must_use]
    pub fn next(&self) -> Option<usize> {
        Some(self.end()).filter(|&end| end < self.matched)
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

    use crate::{
        domain::{
            board::{Object, ObjectKind, query::ObjectQuery, tests::plain_object},
            limit::Limit,
        },
        error::Result,
    };

    fn board() -> [Object; 5] {
        [
            plain_object("s1", ObjectKind::Sticker, "Ёлка, план"),
            plain_object("t1", ObjectKind::Text, "елка"),
            plain_object("s2", ObjectKind::Sticker, "Retro"),
            plain_object("f1", ObjectKind::Frame, "План"),
            plain_object("m1", ObjectKind::Other("mind-map-node".into()), "Идея"),
        ]
    }

    fn shown(query: &ObjectQuery<'_>) -> Result<(Vec<String>, usize)> {
        let board = board();
        let all: Vec<&Object> = board.iter().collect();
        let page = query.page_where(&all, |_| true)?;
        let ids = page.shown.iter().map(|object| object.id.clone()).collect();
        Ok((ids, page.matched))
    }

    fn of_kind(kind: &str) -> Result<(Vec<String>, usize)> {
        shown(&ObjectQuery {
            kind: ObjectKind::typed(kind),
            ..ObjectQuery::default()
        })
    }

    #[test]
    fn kind_takes_the_short_and_the_stored_name() -> Result<()> {
        assert_eq!(of_kind("text")?, (vec!["t1".into()], 1));
        assert_eq!(of_kind(" Simple-Text ")?, (vec!["t1".into()], 1));
        assert_eq!(of_kind("sticker")?.1, 2);
        assert_eq!(of_kind("  ")?.1, 5);
        Ok(())
    }

    #[test]
    fn a_kind_without_variant_finds_the_objects_that_have_it() -> Result<()> {
        assert_eq!(of_kind("Mind-Map-Node")?, (vec!["m1".into()], 1));
        Ok(())
    }

    #[test]
    fn a_kind_with_variant_but_no_object_is_an_empty_page() -> Result<()> {
        assert_eq!(of_kind("dice")?, (Vec::new(), 0));
        Ok(())
    }

    #[test]
    fn a_kind_nobody_has_is_refused_with_the_known_and_the_board_kinds() {
        let refused = of_kind("stikcer").err().map(|err| err.to_string());
        assert!(refused.is_some_and(|text| {
            text.ends_with("flipcard, task, link, mind-map-node")
        }));
    }

    #[test]
    fn text_ignores_case_yo_and_punctuation() -> Result<()> {
        let query = ObjectQuery {
            text: Some("ЕЛКА"),
            ..ObjectQuery::default()
        };
        assert_eq!(shown(&query)?, (vec!["s1".into(), "t1".into()], 2));
        let plan = ObjectQuery {
            text: Some("план"),
            kind: Some(ObjectKind::Frame),
            ..ObjectQuery::default()
        };
        assert_eq!(shown(&plan)?, (vec!["f1".into()], 1));
        Ok(())
    }

    #[test]
    fn offset_and_limit_page_the_matches() -> Result<()> {
        let query = ObjectQuery {
            offset: 1,
            limit: Limit::from(2),
            ..ObjectQuery::default()
        };
        assert_eq!(shown(&query)?, (vec!["t1".into(), "s2".into()], 5));
        let past = ObjectQuery {
            offset: usize::MAX,
            ..ObjectQuery::default()
        };
        assert_eq!(shown(&past)?, (Vec::new(), 5));
        Ok(())
    }
}
