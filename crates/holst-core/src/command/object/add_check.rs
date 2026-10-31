//! The checks of `object add` that need only the arguments given.

use crate::{
    command::{
        object::{AddArgs, AddFields, AddKind},
        table::{ArgumentGroup as _, Validate},
    },
    domain::kinds::{dice, icon, reaction_stamp, table, wheel},
    error::{flag::Flag, input::InputError},
};

/// The whole widths a stroke takes, least and most.
const WIDTH_RANGE: (u32, u32) = (1, 30);

type Checked = Result<(), InputError>;

impl Validate for AddArgs {
    fn validate(&self) -> Checked {
        let kind = self.kind.unwrap_or_default();
        match self.items.as_ref().map(|items| items.0.as_slice()) {
            Some([]) => Err(InputError::Blank { flag: Flag::Items }),
            Some(items) => self
                .add
                .first_given()
                .map_or(Ok(()), |flag| {
                    Err(InputError::Conflict {
                        flag,
                        other: Flag::Items,
                    })
                })
                .and_then(|()| {
                    items.iter().enumerate().try_for_each(|(slot, item)| {
                        let framed = item.frame.is_some() || self.frame.is_some();
                        check(item.kind.unwrap_or(kind), &item.add, framed).map_err(
                            |error| InputError::InItem {
                                slot: slot + 1,
                                error: Box::new(error),
                            },
                        )
                    })
                }),
            None => check(kind, &self.add, self.frame.is_some()),
        }
    }
}

/// The keys of one item: the kinds that take them, then the values they hold.
fn check(kind: AddKind, fields: &AddFields, framed: bool) -> Checked {
    refuse_misplaced(kind, fields, framed)?;
    refuse_values(kind, fields)
}

/// Refuses the first key given for a kind that does not take it.
fn refuse_misplaced(kind: AddKind, fields: &AddFields, framed: bool) -> Checked {
    let arrow = kind == AddKind::Arrow;
    let drawing = kind == AddKind::Drawing;
    let linked = kind == AddKind::Link;
    let uploaded = matches!(kind, AddKind::Image | AddKind::File);
    let wheel = kind == AddKind::Wheel;
    foreign(kind, AddKind::Arrow, &fields.arrow_keys())?;
    foreign(kind, AddKind::Drawing, &fields.drawing_keys())?;
    refuse(
        kind,
        Flag::StrokeWidth,
        !(arrow || drawing) && fields.stroke_width.is_some(),
    )?;
    refuse(kind, Flag::Text, textless(kind) && fields.text.is_some())?;
    refuse(
        kind,
        Flag::Faces,
        kind != AddKind::Dice && fields.faces.is_some(),
    )?;
    refuse(kind, Flag::Choice, !wheel && !fields.choice.is_empty())?;
    refuse(
        kind,
        Flag::Back,
        kind != AddKind::Flipcard && fields.back.is_some(),
    )?;
    refuse_node_keys(kind, fields, framed)?;
    refuse(kind, Flag::At, (arrow || drawing) && fields.at.is_some())?;
    refuse(kind, Flag::Size, sizeless(kind) && fields.size.is_some())?;
    refuse(
        kind,
        Flag::Shape,
        kind != AddKind::Shape && fields.shape.is_some(),
    )?;
    refuse(
        kind,
        Flag::Color,
        colourless(kind) && fields.color.is_some(),
    )?;
    refuse(kind, Flag::Url, !linked && fields.url.is_some())?;
    refuse(kind, Flag::Path, !uploaded && fields.path.is_some())?;
    refuse(kind, Flag::NoPreview, !linked && fields.no_preview)?;
    foreign(kind, AddKind::Frame, &fields.frame_keys())?;
    foreign(kind, AddKind::Code, &fields.code_keys())?;
    foreign(kind, AddKind::Table, &fields.table_keys())?;
    foreign(kind, AddKind::ReactionStamp, &fields.reaction_keys())?;
    foreign(kind, AddKind::Icon, &fields.icon_keys())?;
    foreign(kind, AddKind::Task, &fields.task_keys())?;
    // A task takes the keys of a kanban's cells, for the cell it goes into.
    let cell_keys = fields
        .kanban_keys()
        .map(|(flag, given)| (flag, given && kind != AddKind::Task));
    foreign(kind, AddKind::Kanban, &cell_keys)?;
    [(Flag::At, fields.at.is_some()), (Flag::Frame, framed)]
        .into_iter()
        .find(|(_, given)| *given && fields.kanban.is_some())
        .map_or(Ok(()), |(flag, _)| {
            Err(InputError::Conflict {
                flag,
                other: Flag::Kanban,
            })
        })?;
    (kind == AddKind::ReactionStamp && framed && fields.on.is_some())
        .then_some(InputError::Conflict {
            flag: Flag::Frame,
            other: Flag::On,
        })
        .map_or(Ok(()), Err)
}

fn refuse(kind: AddKind, flag: Flag, given: bool) -> Checked {
    given
        .then_some(InputError::NotFor { flag, kind })
        .map_or(Ok(()), Err)
}

/// Refuses the first given key of `keys`, which only the `owner` kind takes.
fn foreign(kind: AddKind, owner: AddKind, keys: &[(Flag, bool)]) -> Checked {
    keys.iter()
        .find(|(_, given)| *given)
        .filter(|_| kind != owner)
        .map_or(Ok(()), |(flag, _)| {
            Err(InputError::NotFor { flag: *flag, kind })
        })
}

/// Refuses the first key a parent or direction breaks: another kind, a corner
/// or frame by a parent, a direction without one.
fn refuse_node_keys(kind: AddKind, fields: &AddFields, framed: bool) -> Checked {
    let node = kind == AddKind::MindMap;
    let hanging = node && fields.parent.is_some();
    [
        (!node && fields.parent.is_some()).then_some(InputError::NotFor {
            flag: Flag::Parent,
            kind,
        }),
        (!node && fields.direction.is_some()).then_some(InputError::NotFor {
            flag: Flag::Direction,
            kind,
        }),
        (hanging && fields.at.is_some()).then_some(InputError::Conflict {
            flag: Flag::At,
            other: Flag::Parent,
        }),
        (hanging && framed).then_some(InputError::Conflict {
            flag: Flag::Frame,
            other: Flag::Parent,
        }),
        (node && !hanging && fields.direction.is_some()).then_some(
            InputError::Requires {
                flag: Flag::Direction,
                needs: Flag::Parent,
            },
        ),
    ]
    .into_iter()
    .flatten()
    .next()
    .map_or(Ok(()), Err)
}

/// Refuses a value the kind cannot take, whatever the board holds.
fn refuse_values(kind: AddKind, fields: &AddFields) -> Checked {
    match kind {
        AddKind::Drawing => refuse_stroke(fields),
        AddKind::Dice => fields
            .faces
            .filter(|faces| !dice::FACES.contains(faces))
            .map_or(Ok(()), |_| {
                Err(InputError::NotAmongNumbers {
                    flag: Flag::Faces,
                    allowed: &dice::FACES,
                })
            }),
        AddKind::Table => refuse_counts(fields),
        AddKind::Wheel => refuse_choices(&fields.choice),
        AddKind::ReactionStamp => match fields.reaction.as_slice() {
            [key] if !reaction_stamp::KEYS.contains(&key.as_str()) => {
                Err(InputError::NotAmong {
                    flag: Flag::Reaction,
                    allowed: reaction_stamp::KEYS.to_vec(),
                })
            }
            [_, _, ..] => Err(InputError::OneValue {
                flag: Flag::Reaction,
            }),
            [] | [_] => Ok(()),
        },
        AddKind::Icon => fields
            .text
            .as_deref()
            .filter(|name| !icon::is_name(name))
            .map_or(Ok(()), |_| {
                Err(InputError::NotIconName { flag: Flag::Text })
            }),
        AddKind::Code => refuse(kind, Flag::Color, fields.color.is_some()),
        AddKind::Task => refuse_cells(fields),
        AddKind::Sticker
        | AddKind::Text
        | AddKind::Shape
        | AddKind::Frame
        | AddKind::Arrow
        | AddKind::Link
        | AddKind::Image
        | AddKind::File
        | AddKind::StickerStack
        | AddKind::Flipcard
        | AddKind::MindMap
        | AddKind::Document
        | AddKind::Stamp
        | AddKind::Kanban => Ok(()),
    }
}

/// A stroke is two points or more, and a whole width within the web's range.
fn refuse_stroke(fields: &AddFields) -> Checked {
    let points = fields.point.iter().flat_map(|given| &given.0).count();
    let (least, most) = WIDTH_RANGE;
    (points < 2)
        .then_some(InputError::TooFew {
            flag: Flag::Point,
            least: 2,
        })
        .map_or(Ok(()), Err)?;
    fields
        .stroke_width
        .filter(|width| {
            !((f64::from(least)..=f64::from(most)).contains(width)
                && (width - width.round()).abs() < f64::EPSILON)
        })
        .map_or(Ok(()), |_| {
            Err(InputError::NotWhole {
                flag: Flag::StrokeWidth,
                least,
                most,
            })
        })
}

/// A table has at least one row and column, and at most the cell limit.
fn refuse_counts(fields: &AddFields) -> Checked {
    [(Flag::Rows, fields.rows), (Flag::Columns, fields.columns)]
        .into_iter()
        .find(|(_, given)| *given == Some(0))
        .map_or(Ok(()), |(flag, _)| {
            Err(InputError::Below { flag, least: 1 })
        })?;
    let rows = fields.rows.unwrap_or(table::DEFAULT_ROWS);
    let columns = fields.columns.unwrap_or(table::DEFAULT_COLUMNS);
    (rows.saturating_mul(columns) > table::CELL_LIMIT)
        .then_some(InputError::TooManyCells {
            flag: Flag::Rows,
            other: Flag::Columns,
            limit: table::CELL_LIMIT,
        })
        .map_or(Ok(()), Err)
}

/// A wheel has the number of choices and the label lengths the web client
/// takes.
fn refuse_choices(choices: &[String]) -> Checked {
    match choices {
        _ if !(wheel::MIN_CHOICES..=wheel::MAX_CHOICES).contains(&choices.len()) => {
            Err(InputError::ValueCount {
                flag: Flag::Choice,
                least: wheel::MIN_CHOICES,
                most: wheel::MAX_CHOICES,
            })
        }
        _ if choices
            .iter()
            .any(|label| label.chars().count() > wheel::MAX_LABEL) =>
        {
            Err(InputError::LabelTooLong {
                flag: Flag::Choice,
                most: wheel::MAX_LABEL,
            })
        }
        _ => Ok(()),
    }
}

/// The kanban cell a task names: one title each, and only with a kanban.
fn refuse_cells(fields: &AddFields) -> Checked {
    [
        (Flag::Column, &fields.column),
        (Flag::Swimlane, &fields.swimlane),
    ]
    .into_iter()
    .find(|(_, titles)| titles.len() > 1)
    .map_or(Ok(()), |(flag, _)| Err(InputError::OneValue { flag }))?;
    [
        (Flag::Column, &fields.column),
        (Flag::Swimlane, &fields.swimlane),
    ]
    .into_iter()
    .find(|(_, titles)| !titles.is_empty() && fields.kanban.is_none())
    .map_or(Ok(()), |(flag, _)| {
        Err(InputError::Requires {
            flag,
            needs: Flag::Kanban,
        })
    })
}

/// Whether the kind holds rich text; markdown for another kind is refused.
pub(super) const fn rich_kind(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Sticker
            | AddKind::Text
            | AddKind::Shape
            | AddKind::MindMap
            | AddKind::Document
    )
}

/// Whether the kind takes no text.
const fn textless(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Arrow
            | AddKind::Link
            | AddKind::Image
            | AddKind::File
            | AddKind::Dice
            | AddKind::Wheel
            | AddKind::Table
            | AddKind::ReactionStamp
            | AddKind::Kanban
            | AddKind::Drawing
    )
}

/// Whether the kind takes no color.
const fn colourless(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Link
            | AddKind::Image
            | AddKind::File
            | AddKind::Wheel
            | AddKind::Stamp
            | AddKind::Table
            | AddKind::ReactionStamp
            | AddKind::Kanban
    )
}

/// Whether the kind has its own size calls cannot change.
const fn sizeless(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Arrow
            | AddKind::Text
            | AddKind::Link
            | AddKind::File
            | AddKind::Dice
            | AddKind::Wheel
            | AddKind::Flipcard
            | AddKind::MindMap
            | AddKind::Document
            | AddKind::Stamp
            | AddKind::Code
            | AddKind::Table
            | AddKind::ReactionStamp
            | AddKind::Task
            | AddKind::Kanban
            | AddKind::Drawing
    )
}

impl AddFields {
    /// The keys only a code block takes, each with whether it is given.
    fn code_keys(&self) -> [(Flag, bool); 2] {
        [
            (Flag::Language, self.language.is_some()),
            (Flag::Theme, self.theme.is_some()),
        ]
    }

    /// The keys only a table takes, each with whether it is given.
    fn table_keys(&self) -> [(Flag, bool); 2] {
        [
            (Flag::Rows, self.rows.is_some()),
            (Flag::Columns, self.columns.is_some()),
        ]
    }

    /// The keys only an arrow takes, each with whether it is given.
    fn arrow_keys(&self) -> [(Flag, bool); 6] {
        [
            (Flag::From, self.from.is_some()),
            (Flag::To, self.to.is_some()),
            (Flag::Line, self.line.is_some()),
            (Flag::Heads, self.heads.is_some()),
            (Flag::StrokeStyle, self.stroke_style.is_some()),
            (Flag::Label, !self.label.is_empty()),
        ]
    }

    /// The keys only a drawing takes, each with whether it is given.
    fn drawing_keys(&self) -> [(Flag, bool); 2] {
        [
            (Flag::Point, self.point.is_some()),
            (Flag::Highlighter, self.highlighter),
        ]
    }

    /// The keys only a reaction stamp takes, each with whether it is given.
    fn reaction_keys(&self) -> [(Flag, bool); 2] {
        [
            (Flag::On, self.on.is_some()),
            (Flag::Reaction, !self.reaction.is_empty()),
        ]
    }

    /// The keys only an icon takes, each with whether it is given.
    fn icon_keys(&self) -> [(Flag, bool); 1] {
        [(Flag::Weight, self.weight.is_some())]
    }

    /// The keys only a task takes, each with whether it is given.
    fn task_keys(&self) -> [(Flag, bool); 5] {
        [
            (Flag::Description, self.description.is_some()),
            (Flag::Status, self.status.is_some()),
            (Flag::Start, self.start.is_some()),
            (Flag::Due, self.due.is_some()),
            (Flag::Kanban, self.kanban.is_some()),
        ]
    }

    /// The keys only a kanban takes, each with whether it is given.
    fn kanban_keys(&self) -> [(Flag, bool); 2] {
        [
            (Flag::Column, !self.column.is_empty()),
            (Flag::Swimlane, !self.swimlane.is_empty()),
        ]
    }

    /// The keys only a frame takes, each with whether it is given.
    fn frame_keys(&self) -> [(Flag, bool); 3] {
        [
            (Flag::Ratio, self.ratio.is_some()),
            (Flag::Background, self.background.is_some()),
            (Flag::HideContent, self.hide_content),
        ]
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use crate::command::{object::AddArgs, table::parse};

    /// What `parse` says of the arguments, before any request.
    fn parsed(given: Value) -> Option<String> {
        let mut arguments = match given {
            Value::Object(map) => map,
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => serde_json::Map::new(),
        };
        arguments.entry("board").or_insert_with(|| json!("A"));
        parse::<AddArgs>(arguments)
            .err()
            .map(|error| error.to_string())
    }

    #[rstest]
    #[case::empty_items(json!({"items": []}), "`--items` is empty: give a visible value")]
    #[case::fields_beside_items(
        json!({"text": "a", "items": [{"text": "b"}]}),
        "`--text` cannot be used with `--items`"
    )]
    #[case::a_key_of_another_kind(
        json!({"items": [{"text": "a"}, {"text": "b", "from": "x"}]}),
        "`--from` cannot be used for sticker, in item 2"
    )]
    #[case::a_corner_by_a_parent(
        json!({"kind": "mind-map", "text": "a", "parent": "@1", "at": "1,1"}),
        "`--at` cannot be used with `--parent`"
    )]
    #[case::a_direction_without_a_parent(
        json!({"kind": "mind-map", "text": "a", "direction": "down"}),
        "`--direction` needs `--parent`"
    )]
    #[case::a_corner_by_a_kanban(
        json!({"kind": "task", "text": "a", "kanban": "@1", "at": "1,1"}),
        "`--at` cannot be used with `--kanban`"
    )]
    #[case::a_frame_by_an_object_to_stick_on(
        json!({"kind": "reaction-stamp", "on": "@1", "frame": "F"}),
        "`--frame` cannot be used with `--on`"
    )]
    #[case::a_column_without_a_kanban(
        json!({"kind": "task", "text": "a", "column": ["x"]}),
        "`--column` needs `--kanban`"
    )]
    #[case::two_titles_for_a_task(
        json!({"kind": "task", "text": "a", "kanban": "@1", "column": ["a", "b"]}),
        "`--column` takes one value here"
    )]
    #[case::one_point(
        json!({"kind": "drawing", "point": "1,1"}),
        "`--point` needs at least 2 values"
    )]
    #[case::a_stroke_width_with_a_fraction(
        json!({"kind": "drawing", "point": ["1,1", "2,2"], "stroke_width": 2.5}),
        "`--stroke-width` must be a whole number from 1 to 30"
    )]
    #[case::faces_the_web_lacks(
        json!({"kind": "dice", "faces": 7}),
        "`--faces` must be one of 2, 4, 6, 8, 10, 12, 20"
    )]
    #[case::no_rows(
        json!({"kind": "table", "rows": 0}),
        "`--rows` must be at least 1"
    )]
    #[case::too_many_cells(
        json!({"kind": "table", "rows": 60, "columns": 50}),
        "`--rows` times `--columns` is at most 2500 cells"
    )]
    #[case::one_choice(
        json!({"kind": "wheel", "choice": ["a"]}),
        "`--choice` takes 2 to 20 values"
    )]
    #[case::a_long_label(
        json!({"kind": "wheel", "choice": ["a", "b".repeat(41)]}),
        "`--choice` labels are at most 40 characters long"
    )]
    #[case::two_reactions(
        json!({"kind": "reaction-stamp", "reaction": ["like", "heart"]}),
        "`--reaction` takes one value here"
    )]
    #[case::an_unknown_reaction(
        json!({"kind": "reaction-stamp", "reaction": ["wow"]}),
        "`--reaction` must be one of like, heart, star, +1, dislike, figma-question, check, cross"
    )]
    #[case::an_icon_name_with_capitals(
        json!({"kind": "icon", "text": "Rocket"}),
        "`--text` must be an icon name: lower-case letters, digits and dashes, e.g. rocket-launch"
    )]
    #[case::a_color_for_a_code_block(
        json!({"items": [{"kind": "code", "text": "a", "color": "red4"}]}),
        "`--color` cannot be used for code, in item 1"
    )]
    fn parse_refuses_what_the_arguments_alone_refuse(
        #[case] given: Value,
        #[case] expected: &str,
    ) {
        assert_eq!(parsed(given).as_deref(), Some(expected));
    }
}
