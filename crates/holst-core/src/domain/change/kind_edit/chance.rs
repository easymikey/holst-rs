//! Dice and wheels: a roll or a spin, and the faces of a die.

use itertools::Itertools as _;
use serde_json::{Value, json};

use super::{Written, not_changeable, only};
use crate::{
    domain::{
        board::ObjectKind,
        change::{build::author_number, check::kind_of, write::WriteTransaction},
        kinds::{dice::DEFAULT_FACES, wheel::MIN_CHOICES},
    },
    error::{Error, Result, board_change::BoardChangeError},
};

pub(super) const ROLL: &str = "roll";
pub(super) const FACES: &str = "faces";
const VALUE: &str = "value";
const SPIN_TIME: &str = "spinTime";
const ITEMS: &str = "items";
const SELECTED: &str = "selectedItemId";
const EXCLUDED: &str = "excludedItemIds";
const REMOVE_ONCE: &str = "removeItemOnceChosen";
const ROTATION: &str = "wheelRotation";
/// BUNDLE: a spin turns the wheel 5 to 7 whole times before it stops (`S2`).
const FIRST_TURNS: u32 = 5;
const TURN_CHOICES: u32 = 3;

pub(super) fn check_roll(kind: &ObjectKind, fields: Option<&Value>) -> Result<()> {
    let choices = fields.map_or(0, |fields| in_play(fields).1.len());
    match (*kind == ObjectKind::Dice, *kind == ObjectKind::Wheel) {
        (true, _) => Ok(()),
        (false, true) if choices >= MIN_CHOICES => Ok(()),
        (false, true) => Err(Error::BoardChange(BoardChangeError::NotSpinnable {
            choices,
        })),
        (false, false) => Err(not_changeable(kind, ROLL)),
    }
}

pub(super) fn check_faces(kind: &ObjectKind) -> Result<()> {
    only(kind, &ObjectKind::Dice, FACES)
}

/// Rolls the die or spins wheel `id`, drawing a fresh id from the context.
pub(super) fn roll(
    scope: &WriteTransaction<'_>,
    id: &str,
    fields: &Value,
) -> Result<Written> {
    let drawn = scope.context.ids.new_id();
    let draw = |salt| author_number(&drawn, salt);
    let now = json!(scope.context.built_at.as_millis());
    let spinning =
        (kind_of(scope.states, id) == ObjectKind::Wheel).then_some((draw(0), draw(1)));
    let changed = spinning.map_or_else(
        || Ok(vec![(VALUE, json!(rolled(fields, draw(0))))]),
        |draws| spin(fields, draws),
    )?;
    Ok(Written {
        fields: changed.into_iter().chain([(SPIN_TIME, now)]).collect(),
        touched: Vec::new(),
    })
}

/// The die's new face up: 1 to its faces.
fn rolled(die: &Value, draw: u32) -> u32 {
    1 + draw.checked_rem(faces_of(die)).unwrap_or_default()
}

fn faces_of(die: &Value) -> u32 {
    die.get(FACES)
        .and_then(Value::as_u64)
        .and_then(|faces| u32::try_from(faces).ok())
        .unwrap_or(DEFAULT_FACES)
}

/// The faces of a die; the value stays among them.
pub(super) fn faces(faces: u32, die: &Value) -> Written {
    let held = die
        .get(VALUE)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1);
    Written {
        fields: vec![
            (FACES, json!(faces)),
            (VALUE, json!(held.clamp(1, faces.max(1)))),
        ],
        touched: Vec::new(),
    }
}

/// Spin start ids (plus last winner if once-only) and the choices in play.
fn in_play(wheel: &Value) -> (Vec<String>, Vec<String>) {
    let strings = |key: &str, name: Option<&str>| -> Vec<String> {
        wheel
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| name.map_or(Some(entry), |name| entry.get(name)))
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let last = wheel
        .get(REMOVE_ONCE)
        .and_then(Value::as_bool)
        .filter(|once| *once)
        .and_then(|_| wheel.get(SELECTED))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let out: Vec<String> = strings(EXCLUDED, None)
        .into_iter()
        .chain(last)
        .unique()
        .collect();
    let live = strings(ITEMS, Some("id"))
        .into_iter()
        .filter(|item| !out.contains(item))
        .collect();
    (out, live)
}

/// Stops the wheel on a random choice in play, turning from where it stood.
fn spin(
    wheel: &Value,
    (pick, turns): (u32, u32),
) -> Result<Vec<(&'static str, Value)>> {
    let (out, live) = in_play(wheel);
    let choices = live.len();
    let count = u32::try_from(choices).unwrap_or_default();
    let slot = pick.checked_rem(count).unwrap_or_default();
    let winner = usize::try_from(slot)
        .ok()
        .and_then(|slot| live.get(slot))
        .filter(|_| choices >= MIN_CHOICES)
        .ok_or(Error::BoardChange(BoardChangeError::NotSpinnable {
            choices,
        }))?;
    let sector = 360.0 / f64::from(count);
    // BUNDLE: the winner's centre turns to the top (S2: S(i, n)).
    let stop = (-sector * (f64::from(slot) + 0.5)).rem_euclid(360.0);
    let from = wheel
        .get(ROTATION)
        .and_then(Value::as_f64)
        .unwrap_or_default();
    let whole = FIRST_TURNS + turns.checked_rem(TURN_CHOICES).unwrap_or_default();
    let to = from + f64::from(whole) * 360.0 + (stop - from).rem_euclid(360.0);
    Ok(vec![
        (EXCLUDED, json!(out)),
        (SELECTED, json!(winner)),
        (ROTATION, json!(to)),
    ])
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use crate::{
        domain::{
            board,
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{
                    BoardChange,
                    Made,
                    NewEntry,
                    NewObject,
                    ObjectEdit,
                    ObjectPatch,
                },
            },
            geometry::Point,
            id_source::{IdSource as _, SequentialIds},
            kinds::{dice, wheel},
            rich_text::TextFormat,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    fn board_with(new: NewObject) -> Result<Doc> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new,
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn a_die(extra: &[(&'static str, Value)]) -> Result<Doc> {
        let mut made = dice::made(20, None);
        made.fields.extend(extra.iter().cloned());
        board_with(NewObject::Made(made))
    }

    fn a_wheel(extra: &[(&'static str, Value)]) -> Result<Doc> {
        let ids = SequentialIds::new("c");
        let choices = ["Pizza", "Sushi", "Soup"].map(str::to_owned);
        let mut made: Made = wheel::made(&choices, || ids.new_id());
        made.fields.extend(extra.iter().cloned());
        board_with(NewObject::Made(made))
    }

    fn edit(document: &Doc, kind_edit: KindEdit) -> Result<()> {
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: "s-1".into(),
            patch: ObjectPatch {
                kind_edits: vec![kind_edit],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new("r"), &change).map(drop)
    }

    fn stored(document: &Doc, names: [&str; 4]) -> [Option<Value>; 4] {
        let transaction = document.transact();
        let fields = transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, "s-1"))
            .and_then(|value| board::json(&value.to_json(&transaction)));
        names.map(|name| fields.as_ref().and_then(|fields| fields.get(name).cloned()))
    }

    const DIE: [&str; 4] = ["faces", "value", "spinTime", "updated"];
    const WHEEL: [&str; 4] = [
        "selectedItemId",
        "excludedItemIds",
        "wheelRotation",
        "spinTime",
    ];

    #[test]
    fn a_die_rolled_with_a_fixed_id_gets_its_value_and_the_time() -> Result<()> {
        let document = a_die(&[])?;
        edit(&document, KindEdit::Roll)?;
        let [faces, value, spin, _] = stored(&document, DIE);
        assert_eq!(
            (faces, value, spin),
            (Some(json!(20)), Some(json!(6)), Some(json!(2)))
        );
        Ok(())
    }

    #[test]
    fn faces_change_keeps_the_value_in_range() -> Result<()> {
        let fewer = a_die(&[("value", json!(15))])?;
        edit(&fewer, KindEdit::Faces(6))?;
        let more = a_die(&[("value", json!(3))])?;
        edit(&more, KindEdit::Faces(20))?;
        let [faces, value, _, _] = stored(&fewer, DIE);
        let [more_faces, more_value, _, _] = stored(&more, DIE);
        assert_eq!(
            [(faces, value), (more_faces, more_value)],
            [
                (Some(json!(6)), Some(json!(6))),
                (Some(json!(20)), Some(json!(3)))
            ]
        );
        Ok(())
    }

    #[test]
    fn a_wheel_spun_with_a_fixed_id_stops_at_a_choice() -> Result<()> {
        let document = a_wheel(&[])?;
        edit(&document, KindEdit::Roll)?;
        assert_eq!(
            stored(&document, WHEEL),
            [
                Some(json!("c-3")),
                Some(json!([])),
                Some(json!(1860)),
                Some(json!(2))
            ]
        );
        Ok(())
    }

    #[test]
    fn a_wheel_with_one_live_choice_is_refused() -> Result<()> {
        let document = a_wheel(&[("excludedItemIds", json!(["c-1", "c-2"]))])?;
        let refused = edit(&document, KindEdit::Roll);
        assert!(matches!(
            refused,
            Err(Error::BoardChange(BoardChangeError::NotSpinnable {
                choices: 1
            }))
        ));
        Ok(())
    }

    #[test]
    fn the_last_winner_is_excluded_when_choices_go_once_chosen() -> Result<()> {
        let document = a_wheel(&[
            ("removeItemOnceChosen", json!(true)),
            ("selectedItemId", json!("c-1")),
        ])?;
        edit(&document, KindEdit::Roll)?;
        let [chosen, excluded, _, _] = stored(&document, WHEEL);
        assert_eq!(
            (chosen, excluded),
            (Some(json!("c-3")), Some(json!(["c-1"])))
        );
        Ok(())
    }

    #[test]
    fn a_sticker_takes_neither() -> Result<()> {
        let document = board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })?;
        let refused = [KindEdit::Roll, KindEdit::Faces(6)]
            .map(|kind_edit| edit(&document, kind_edit).err());
        assert!(matches!(
            refused,
            [
                Some(Error::BoardChange(BoardChangeError::NotChangeable {
                    field: "roll",
                    ..
                })),
                Some(Error::BoardChange(BoardChangeError::NotChangeable {
                    field: "faces",
                    ..
                })),
            ]
        ));
        Ok(())
    }
}
