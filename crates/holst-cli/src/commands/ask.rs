//! What a held run asks, and how the answer goes back into its arguments.

use holst_core::error::confirmation::Unconfirmed;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    form::{Form, terminal::Terminal},
};

/// What an error asks a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Question {
    Confirm(Unconfirmed),
}

/// The question a run's error asks, if it asks one.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only a held change asks; every other error ends the run as it is"
)]
pub(crate) fn question(error: &Error) -> Option<Question> {
    match error {
        Error::Unconfirmed(needs) => Some(Question::Confirm(needs.clone())),
        _ => None,
    }
}

/// The arguments of the run that repeats the one that asked `question`, after
/// a Yes.
#[must_use]
pub(crate) fn answered(
    mut arguments: Map<String, Value>,
    question: &Question,
) -> Map<String, Value> {
    match question {
        Question::Confirm(_) => {
            arguments.insert("confirm".to_owned(), Value::Bool(true));
            arguments
        }
    }
}

/// Shows the question and reads the answer; `Ok` is a Yes. A No is the
/// refusal the run ended with, already shown.
pub(crate) fn put<T: Terminal>(form: &Form<T>, question: &Question) -> Result<()> {
    match question {
        Question::Confirm(needs) => form
            .confirm(&format!("{}? (y/N)", needs.needs()), false)
            .and_then(|yes| {
                yes.then_some(())
                    .ok_or_else(|| Error::Unconfirmed(needs.clone()).shown())
            }),
    }
}

#[cfg(test)]
mod tests {
    use holst_core::error::confirmation::{ChangeRisk, Unconfirmed};
    use rstest::rstest;
    use serde_json::{Value, json};

    use crate::{
        commands::ask::{Question, answered, put, question},
        error::{Error, Result},
        form::{Form, fake::Script, line::Keystroke, terminal::TerminalKind},
    };

    fn needs() -> Unconfirmed {
        Unconfirmed::new("deleting a board", ChangeRisk::Irreversible)
    }

    fn outcome(result: &Result<()>) -> &'static str {
        [
            (result.is_ok(), "yes"),
            (
                matches!(result, Err(Error::Shown(inner)) if matches!(**inner, Error::Unconfirmed(_))),
                "no",
            ),
            (matches!(result, Err(Error::Interrupted)), "interrupted"),
            (matches!(result, Err(Error::Cancelled)), "cancelled"),
        ]
        .into_iter()
        .find_map(|(hit, name)| hit.then_some(name))
        .unwrap_or("other")
    }

    #[rstest]
    #[case::unconfirmed(Error::Unconfirmed(needs()), Some(Question::Confirm(needs())))]
    #[case::cancelled(Error::Cancelled, None)]
    #[case::shown(Error::Unconfirmed(needs()).shown(), None)]
    fn only_a_held_change_asks(#[case] error: Error, #[case] asked: Option<Question>) {
        assert_eq!(question(&error), asked);
    }

    #[test]
    fn a_yes_adds_the_confirmation_and_keeps_the_rest() {
        let arguments = json!({ "board": "Plans" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let next = answered(arguments, &Question::Confirm(needs()));
        assert_eq!(
            Value::Object(next),
            json!({ "board": "Plans", "confirm": true })
        );
    }

    #[rstest]
    #[case::y_key(TerminalKind::Live, vec![Keystroke::Char('y')], &[], "yes")]
    #[case::n_key(TerminalKind::Live, vec![Keystroke::Char('n')], &[], "no")]
    #[case::enter_defaults_to_no(TerminalKind::Live, vec![Keystroke::Enter], &[], "no")]
    #[case::tab_then_enter(
        TerminalKind::Live,
        vec![Keystroke::Tab, Keystroke::Enter],
        &[],
        "yes"
    )]
    #[case::ctrl_c(TerminalKind::Live, vec![Keystroke::Interrupt], &[], "interrupted")]
    #[case::escape(TerminalKind::Live, vec![Keystroke::Cancel], &[], "cancelled")]
    #[case::ctrl_d(TerminalKind::Live, vec![Keystroke::EndOfInput], &[], "cancelled")]
    #[case::line_yes(TerminalKind::Dumb, vec![], &["yes"], "yes")]
    #[case::line_no(TerminalKind::Dumb, vec![], &["No"], "no")]
    #[case::line_maybe(TerminalKind::Dumb, vec![], &["maybe"], "no")]
    #[case::line_empty(TerminalKind::Dumb, vec![], &[""], "no")]
    #[case::line_end_of_input(TerminalKind::Dumb, vec![], &[], "cancelled")]
    #[case::piped(TerminalKind::Redirected, vec![], &["y"], "other")]
    fn the_answer_is_the_exit(
        #[case] kind: TerminalKind,
        #[case] strokes: Vec<Keystroke>,
        #[case] lines: &'static [&'static str],
        #[case] expected: &str,
    ) {
        let form = Form::bare(Script::new(kind, strokes, lines.iter().copied()));
        let result = put(&form, &Question::Confirm(needs()));
        assert_eq!(outcome(&result), expected, "{result:?}");
    }

    #[test]
    fn the_question_names_the_change_and_the_answer_stays() {
        let form =
            Form::bare(Script::new(TerminalKind::Live, [Keystroke::Char('y')], []));
        let result = put(&form, &Question::Confirm(needs()));
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
}
