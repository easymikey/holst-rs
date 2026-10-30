//! What a held run asks, and how the answer goes back into its arguments.

use holst_core::error::{
    confirmation::Unconfirmed,
    input::InputError,
    lookup::{Candidate, Candidates, LookupError, LookupMiss},
};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    form::{Form, prompt::Choice, terminal::Terminal},
};

/// The argument an answer replaces: `key`, or element `at` of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    key: String,
    at: Option<usize>,
}

/// What an error asks a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Question {
    Confirm(Unconfirmed),
    /// Each option pairs the id that goes into the slot with what is shown.
    Choose {
        slot: Slot,
        label: String,
        options: Vec<(String, Choice)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    Confirmed,
    Chosen(Slot, String),
}

/// The question a run's error asks, if it asks one.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only a held change or a choice asks; every other error ends the run as it is"
)]
pub(crate) fn question(
    error: &Error,
    arguments: &Map<String, Value>,
) -> Option<Question> {
    match error {
        Error::Unconfirmed(needs) => Some(Question::Confirm(needs.clone())),
        Error::Holst(holst_core::Error::Lookup(LookupError {
            kind,
            reference,
            miss: LookupMiss::Ambiguous { candidates, .. },
        })) => slot_of(arguments, reference).and_then(|slot| {
            choose(
                slot,
                format!("Which {kind} is \"{reference}\"?"),
                Vec::new(),
                candidates,
            )
        }),
        Error::Holst(holst_core::Error::Input(InputError::WorkspaceNotChosen {
            workspaces,
        })) => choose(
            Slot {
                key: "workspace".to_owned(),
                at: None,
            },
            "Which workspace?".to_owned(),
            Vec::new(),
            workspaces,
        ),
        Error::Holst(holst_core::Error::Input(InputError::RootOrFolder {
            reference,
            folders,
        })) => slot_of(arguments, reference).and_then(|slot| {
            choose(
                slot,
                format!("\"{reference}\" is the workspace root and a folder; which?"),
                vec![(
                    "/".to_owned(),
                    Choice {
                        label: "the workspace root".to_owned(),
                        hint: Some("/".to_owned()),
                    },
                )],
                folders,
            )
        }),
        _ => None,
    }
}

/// The one argument, or element of one, that is `reference`.
fn slot_of(arguments: &Map<String, Value>, reference: &str) -> Option<Slot> {
    let slots: Vec<Slot> = arguments
        .iter()
        .flat_map(|(key, value)| match value {
            Value::String(text) if text == reference => vec![Slot {
                key: key.clone(),
                at: None,
            }],
            Value::Array(items) => items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.as_str() == Some(reference))
                .map(|(at, _)| Slot {
                    key: key.clone(),
                    at: Some(at),
                })
                .collect(),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Object(_) => Vec::new(),
        })
        .collect();
    <[Slot; 1]>::try_from(slots).ok().map(|[only]| only)
}

/// Asks to pick among `first` and the candidates, when the list is whole and
/// each candidate has an id.
fn choose(
    slot: Slot,
    label: String,
    first: Vec<(String, Choice)>,
    candidates: &Candidates,
) -> Option<Question> {
    let rest: Option<Vec<(String, Choice)>> = candidates
        .listed
        .iter()
        .map(|candidate| candidate.id.clone().map(|id| (id, shown(candidate))))
        .collect();
    rest.filter(|_| candidates.more == 0)
        .map(|rest| Question::Choose {
            slot,
            label,
            options: first.into_iter().chain(rest).collect(),
        })
}

/// The candidate by its place and name; the id, when it adds anything, is the
/// hint.
fn shown(candidate: &Candidate) -> Choice {
    let place: String = candidate
        .workspace
        .iter()
        .chain(&candidate.folders)
        .flat_map(|part| [part.as_str(), "/"])
        .collect();
    let slash = ["", "/"]
        .get(usize::from(candidate.folder))
        .copied()
        .unwrap_or_default();
    Choice {
        label: format!("{place}{}{slash}", candidate.name),
        hint: candidate.id.clone().filter(|id| *id != candidate.name),
    }
}

/// The arguments of the run that repeats the one that asked, after `answer`.
#[must_use]
pub(crate) fn answered(
    mut arguments: Map<String, Value>,
    answer: Answer,
) -> Map<String, Value> {
    match answer {
        Answer::Confirmed => {
            arguments.insert("confirm".to_owned(), Value::Bool(true));
        }
        Answer::Chosen(Slot { key, at }, id) => match (at, arguments.get_mut(&key)) {
            (Some(place), Some(Value::Array(items))) => items
                .iter_mut()
                .skip(place)
                .take(1)
                .for_each(|item| *item = Value::String(id.clone())),
            (None | Some(_), _) => {
                arguments.insert(key, Value::String(id));
            }
        },
    }
    arguments
}

/// Shows the question and reads the answer. A No is the refusal the run ended
/// with, already shown.
pub(crate) fn put<T: Terminal>(form: &Form<T>, question: Question) -> Result<Answer> {
    match question {
        Question::Confirm(needs) => form
            .confirm(&format!("{}? (y/N)", needs.needs()), false)
            .map_err(Error::from)
            .and_then(|yes| {
                yes.then_some(Answer::Confirmed)
                    .ok_or_else(|| Error::Unconfirmed(needs).shown())
            }),
        Question::Choose {
            slot,
            label,
            options,
        } => {
            let (ids, choices): (Vec<String>, Vec<Choice>) =
                options.into_iter().unzip();
            form.select(&label, choices)
                .map_err(Error::from)
                .and_then(|at| {
                    ids.into_iter()
                        .nth(at)
                        .map(|id| Answer::Chosen(slot, id))
                        .ok_or(Error::Cancelled)
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use holst_core::error::{
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
        lookup::{
            Candidate,
            Candidates,
            LookupError,
            LookupKind,
            LookupMiss,
            SkippedWorkspaces,
        },
    };
    use rstest::rstest;
    use serde_json::{Map, Value, json};

    use crate::{
        commands::ask::{Answer, Question, Slot, answered, put, question},
        error::{Error, Result},
        form::{
            Form,
            fake::Script,
            line::Keystroke,
            prompt::Choice,
            terminal::TerminalKind,
        },
    };

    fn needs() -> Unconfirmed {
        Unconfirmed::new("deleting a board", ChangeRisk::Irreversible)
    }

    fn arguments(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    fn candidate(name: &str, id: Option<&str>) -> Candidate {
        Candidate {
            name: name.to_owned(),
            id: id.map(str::to_owned),
            ..Candidate::default()
        }
    }

    fn candidates<const N: usize>(items: [Candidate; N]) -> Candidates {
        items.into_iter().collect()
    }

    fn two_boards() -> Candidates {
        candidates([
            candidate("Plans", Some("b1")),
            Candidate {
                workspace: Some("easy".to_owned()),
                folders: vec!["Q4".to_owned()],
                ..candidate("Plans", Some("b2"))
            },
        ])
    }

    fn lookup(kind: LookupKind, reference: &str, miss: LookupMiss) -> Error {
        Error::Holst(holst_core::Error::Lookup(LookupError {
            kind,
            reference: reference.to_owned(),
            miss,
        }))
    }

    fn input(error: InputError) -> Error {
        Error::Holst(holst_core::Error::Input(error))
    }

    fn slot(key: &str, at: Option<usize>) -> Slot {
        Slot {
            key: key.to_owned(),
            at,
        }
    }

    fn option(id: &str, label: &str, hint: Option<&str>) -> (String, Choice) {
        (
            id.to_owned(),
            Choice {
                label: label.to_owned(),
                hint: hint.map(str::to_owned),
            },
        )
    }

    fn choose(slot: Slot, label: &str, options: Vec<(String, Choice)>) -> Question {
        Question::Choose {
            slot,
            label: label.to_owned(),
            options,
        }
    }

    fn boards_asked(kind: &str) -> Question {
        choose(
            slot("board", None),
            &format!("Which {kind} is \"Plans\"?"),
            vec![
                option("b1", "Plans", Some("b1")),
                option("b2", "easy/Q4/Plans", Some("b2")),
            ],
        )
    }

    #[rstest]
    #[case::unconfirmed(
        Error::Unconfirmed(needs()),
        json!({ "board": "Plans" }),
        Some(Question::Confirm(needs()))
    )]
    #[case::cancelled(Error::Cancelled, json!({}), None)]
    #[case::shown(Error::Unconfirmed(needs()).shown(), json!({}), None)]
    #[case::several_boards(
        lookup(LookupKind::Board, "Plans", LookupMiss::ambiguous(two_boards())),
        json!({ "board": "Plans", "permanently": true }),
        Some(boards_asked("board"))
    )]
    #[case::element_of_a_list(
        lookup(
            LookupKind::Object,
            "abc",
            LookupMiss::ambiguous(candidates([
                candidate("abc1", Some("abc1")),
                candidate("abc2", Some("abc2"))
            ]))
        ),
        json!({ "object": ["x", "abc"] }),
        Some(choose(
            slot("object", Some(1)),
            "Which object is \"abc\"?",
            vec![option("abc1", "abc1", None), option("abc2", "abc2", None)]
        ))
    )]
    #[case::more_than_listed(
        lookup(
            LookupKind::Board,
            "Plans",
            LookupMiss::ambiguous(Candidates { more: 3, ..two_boards() })
        ),
        json!({ "board": "Plans" }),
        None
    )]
    #[case::candidate_without_id(
        lookup(
            LookupKind::Board,
            "Plans",
            LookupMiss::ambiguous(candidates([
                candidate("Plans", None),
                candidate("Plans", Some("b2"))
            ]))
        ),
        json!({ "board": "Plans" }),
        None
    )]
    #[case::reference_given_twice(
        lookup(LookupKind::Board, "Plans", LookupMiss::ambiguous(two_boards())),
        json!({ "board": "Plans", "to": "Plans" }),
        None
    )]
    #[case::reference_not_an_argument(
        lookup(LookupKind::Board, "Plans", LookupMiss::ambiguous(two_boards())),
        json!({ "board": "Other" }),
        None
    )]
    #[case::uncertain(
        lookup(
            LookupKind::Board,
            "Plans",
            LookupMiss::Uncertain {
                candidates: two_boards(),
                unread: SkippedWorkspaces::default()
            }
        ),
        json!({ "board": "Plans" }),
        None
    )]
    #[case::not_found(
        lookup(LookupKind::Board, "Plans", LookupMiss::not_found()),
        json!({ "board": "Plans" }),
        None
    )]
    #[case::frame(
        lookup(LookupKind::Frame, "Plans", LookupMiss::ambiguous(two_boards())),
        json!({ "frame": "Plans" }),
        Some(Question::Choose {
            slot: slot("frame", None),
            label: "Which frame is \"Plans\"?".to_owned(),
            options: vec![
                option("b1", "Plans", Some("b1")),
                option("b2", "easy/Q4/Plans", Some("b2"))
            ]
        })
    )]
    #[case::workspace_not_chosen(
        input(InputError::WorkspaceNotChosen {
            workspaces: candidates([
                candidate("easy", Some("w1")),
                candidate("team", Some("w2"))
            ])
        }),
        json!({ "name": "Plans" }),
        Some(choose(
            slot("workspace", None),
            "Which workspace?",
            vec![option("w1", "easy", Some("w1")), option("w2", "team", Some("w2"))]
        ))
    )]
    #[case::root_or_folder(
        input(InputError::RootOrFolder {
            reference: "root".to_owned(),
            folders: candidates([candidate("Root", Some("f1"))])
        }),
        json!({ "folder": "root" }),
        Some(choose(
            slot("folder", None),
            "\"root\" is the workspace root and a folder; which?",
            vec![
                option("/", "the workspace root", Some("/")),
                option("f1", "Root", Some("f1"))
            ]
        ))
    )]
    fn only_a_held_change_or_a_choice_asks(
        #[case] error: Error,
        #[case] given: Value,
        #[case] asked: Option<Question>,
    ) {
        assert_eq!(question(&error, &arguments(&given)), asked);
    }

    #[rstest]
    #[case::confirmed(
        Answer::Confirmed,
        json!({ "board": "Plans" }),
        json!({ "board": "Plans", "confirm": true })
    )]
    #[case::name_replaced_by_id(
        Answer::Chosen(slot("board", None), "b2".to_owned()),
        json!({ "board": "Plans", "permanently": true }),
        json!({ "board": "b2", "permanently": true })
    )]
    #[case::element_replaced(
        Answer::Chosen(slot("object", Some(1)), "abc2".to_owned()),
        json!({ "object": ["x", "abc"] }),
        json!({ "object": ["x", "abc2"] })
    )]
    #[case::workspace_added(
        Answer::Chosen(slot("workspace", None), "w2".to_owned()),
        json!({ "name": "Plans" }),
        json!({ "name": "Plans", "workspace": "w2" })
    )]
    fn an_answer_goes_into_its_slot_and_the_rest_stays(
        #[case] answer: Answer,
        #[case] given: Value,
        #[case] expected: Value,
    ) {
        assert_eq!(Value::Object(answered(arguments(&given), answer)), expected);
    }

    fn outcome(result: &Result<Answer>) -> String {
        match result {
            Ok(Answer::Confirmed) => "yes".to_owned(),
            Ok(Answer::Chosen(_, id)) => format!("chose {id}"),
            Err(Error::Shown(inner)) if matches!(**inner, Error::Unconfirmed(_)) => {
                "no".to_owned()
            }
            Err(Error::Interrupted) => "interrupted".to_owned(),
            Err(Error::Cancelled) => "cancelled".to_owned(),
            Err(_) => "other".to_owned(),
        }
    }

    fn confirm() -> Question {
        Question::Confirm(needs())
    }

    #[rstest]
    #[case::y_key(TerminalKind::Live, vec![Keystroke::Char('y')], &[], confirm(), "yes")]
    #[case::n_key(TerminalKind::Live, vec![Keystroke::Char('n')], &[], confirm(), "no")]
    #[case::enter_defaults_to_no(TerminalKind::Live, vec![Keystroke::Enter], &[], confirm(), "no")]
    #[case::tab_then_enter(
        TerminalKind::Live,
        vec![Keystroke::Tab, Keystroke::Enter],
        &[],
        confirm(),
        "yes"
    )]
    #[case::ctrl_c(TerminalKind::Live, vec![Keystroke::Interrupt], &[], confirm(), "interrupted")]
    #[case::escape(TerminalKind::Live, vec![Keystroke::Cancel], &[], confirm(), "cancelled")]
    #[case::ctrl_d(TerminalKind::Live, vec![Keystroke::EndOfInput], &[], confirm(), "cancelled")]
    #[case::line_yes(TerminalKind::Dumb, vec![], &["yes"], confirm(), "yes")]
    #[case::line_no(TerminalKind::Dumb, vec![], &["No"], confirm(), "no")]
    #[case::line_maybe(TerminalKind::Dumb, vec![], &["maybe"], confirm(), "no")]
    #[case::line_empty(TerminalKind::Dumb, vec![], &[""], confirm(), "no")]
    #[case::line_end_of_input(TerminalKind::Dumb, vec![], &[], confirm(), "cancelled")]
    #[case::piped(TerminalKind::Redirected, vec![], &["y"], confirm(), "other")]
    #[case::first_board(TerminalKind::Live, vec![Keystroke::Enter], &[], boards_asked("board"), "chose b1")]
    #[case::second_board(
        TerminalKind::Live,
        vec![Keystroke::Down, Keystroke::Enter],
        &[],
        boards_asked("board"),
        "chose b2"
    )]
    #[case::choice_escape(TerminalKind::Live, vec![Keystroke::Cancel], &[], boards_asked("board"), "cancelled")]
    #[case::choice_ctrl_c(TerminalKind::Live, vec![Keystroke::Interrupt], &[], boards_asked("board"), "interrupted")]
    #[case::numbered_line(TerminalKind::Dumb, vec![], &["2"], boards_asked("board"), "chose b2")]
    #[case::number_out_of_range(TerminalKind::Dumb, vec![], &["3"], boards_asked("board"), "cancelled")]
    #[case::choice_piped(TerminalKind::Redirected, vec![], &["2"], boards_asked("board"), "other")]
    fn the_answer_is_the_exit(
        #[case] kind: TerminalKind,
        #[case] strokes: Vec<Keystroke>,
        #[case] lines: &'static [&'static str],
        #[case] asked: Question,
        #[case] expected: &str,
    ) {
        let form = Form::bare(Script::new(kind, strokes, lines.iter().copied()));
        let result = put(&form, asked);
        assert_eq!(outcome(&result), expected, "{result:?}");
    }

    #[test]
    fn the_question_names_the_change_and_the_answer_stays() {
        let form =
            Form::bare(Script::new(TerminalKind::Live, [Keystroke::Char('y')], []));
        let result = put(&form, confirm());
        let written = form.terminal().written();
        assert!(result.is_ok(), "{result:?}");
        assert!(
            written.contains("◆  deleting a board cannot be undone? (y/N)\n"),
            "{written:?}"
        );
        assert!(
            written
                .ends_with("◇  deleting a board cannot be undone? (y/N)\n│  Yes\n│\n"),
            "{written:?}"
        );
    }

    #[test]
    fn the_choice_shows_the_places_and_keeps_the_pick() {
        let form = Form::bare(Script::new(
            TerminalKind::Live,
            [Keystroke::Down, Keystroke::Enter],
            [],
        ));
        let result = put(&form, boards_asked("board"));
        let written = form.terminal().written();
        assert_eq!(outcome(&result), "chose b2");
        assert!(
            written.contains("◆  Which board is \"Plans\"?\n"),
            "{written:?}"
        );
        assert!(written.contains("○ easy/Q4/Plans (b2)"), "{written:?}");
        assert!(
            written.ends_with("◇  Which board is \"Plans\"?\n│  easy/Q4/Plans\n│\n"),
            "{written:?}"
        );
    }
}
