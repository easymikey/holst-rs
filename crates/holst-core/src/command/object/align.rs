//! `object align`: line objects up against a side, or spread them evenly.
use crate::{
    command::{
        object::{AlignArgs, change::named, group::written},
        table::Validate,
    },
    domain::change::{
        receipt::ChangeReport,
        value::{Alignment, Arrangement, BoardChange},
    },
    effect::settings::CacheSettings,
    error::{Result, flag::Flag, input::InputError},
    remote::client::Client,
};

const ALIGN: &str = "object align";

/// A side or a spread, not both and not neither; a side must be one there is.
impl Validate for AlignArgs {
    fn validate(&self) -> std::result::Result<(), InputError> {
        arrangement(self).map(drop)
    }
}

/// Aligns the objects to one side of the box they cover together, or spreads
/// them so that the gaps between are equal; each moves with what it holds.
///
/// # Errors
/// No object, an object that is not found, neither or both of a side and a
/// spread, a locked object, a table cell, a mind-map node or a card in a kanban
/// column, too few objects that can be moved, a board that is not found or not
/// editable, and errors of the requests and of the write.
pub async fn align(
    client: &Client,
    cache: &CacheSettings,
    args: &AlignArgs,
) -> Result<ChangeReport> {
    let how = arrangement(args)?;
    let objects = named(&args.board, &args.object, ALIGN)?;
    written(
        client,
        cache,
        args.dry_run,
        (&args.board, args.workspace.as_deref()),
        BoardChange::Arrange { objects, how },
        None,
    )
    .await
}

/// What `args` ask: a side, or a direction to spread in, not both.
fn arrangement(args: &AlignArgs) -> std::result::Result<Arrangement, InputError> {
    match (&args.side, args.distribute) {
        (Some(side), None) => side
            .trim()
            .parse::<Alignment>()
            .map(Arrangement::Align)
            .map_err(|_| InputError::NotAlignSide { raw: side.clone() }),
        (None, Some(axis)) => Ok(Arrangement::Distribute(axis)),
        (None, None) => Err(InputError::OneOf {
            flags: &[Flag::Side, Flag::Distribute],
        }),
        (Some(_), Some(_)) => Err(InputError::Conflict {
            flag: Flag::Distribute,
            other: Flag::Side,
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::{
        Error,
        command::{fixture::arguments, object::AlignArgs},
        error::{flag::Flag, input::InputError},
    };

    fn refused(given: Value) -> Option<InputError> {
        match arguments::<AlignArgs>(given) {
            Err(Error::Input(error)) => Some(error),
            Err(_) | Ok(_) => None,
        }
    }

    #[test]
    fn parse_refuses_neither_both_and_a_wrong_side() {
        assert!(matches!(
            refused(json!({ "object": ["a", "b"] })),
            Some(InputError::OneOf { .. })
        ));
        assert!(matches!(
            refused(
                json!({ "object": ["a", "b"], "side": "left", "distribute": "horizontal" })
            ),
            Some(InputError::Conflict {
                flag: Flag::Distribute,
                other: Flag::Side
            })
        ));
        assert!(matches!(
            refused(json!({ "object": ["a", "b"], "side": "up" })),
            Some(InputError::NotAlignSide { .. })
        ));
    }
}
