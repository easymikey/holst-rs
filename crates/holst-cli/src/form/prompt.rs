//! Prompts as pure machines: a key gives the next [`Step`]; no IO, no clock.

use crate::{
    form::line::{Cancellation, Keystroke},
    output::coloring::Coloring,
};

/// What a key did to a prompt `M` whose answer is `A`.
#[derive(Debug)]
pub(crate) enum Step<M, A> {
    Editing(M),
    Submit(A),
    Quit(Cancellation),
}

pub(crate) trait Prompt: Sized {
    type Answer;

    #[must_use]
    fn press(self, key: Keystroke) -> Step<Self, Self::Answer>;

    /// The rows under the label while the prompt is active; no escapes for
    /// moves.
    #[must_use]
    fn view(&self, coloring: Coloring, width: Option<usize>) -> Vec<String>;

    /// The text of the line that stays once the prompt is submitted.
    #[must_use]
    fn summary(&self) -> String;
}

/// Yes or No; Enter takes the highlighted one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Confirm {
    yes: bool,
}

impl Confirm {
    #[must_use]
    pub(crate) const fn new(default: bool) -> Self {
        Self { yes: default }
    }
}

impl Prompt for Confirm {
    type Answer = bool;

    fn press(self, key: Keystroke) -> Step<Self, bool> {
        match key {
            Keystroke::Left
            | Keystroke::Right
            | Keystroke::Tab
            | Keystroke::Char('h' | 'l') => Step::Editing(Self { yes: !self.yes }),
            Keystroke::Char('y' | 'Y') => Step::Submit(true),
            Keystroke::Char('n' | 'N') => Step::Submit(false),
            Keystroke::Enter => Step::Submit(self.yes),
            Keystroke::Interrupt => Step::Quit(Cancellation::Interrupt),
            Keystroke::Cancel | Keystroke::EndOfInput => {
                Step::Quit(Cancellation::Cancel)
            }
            Keystroke::Char(_)
            | Keystroke::Backspace
            | Keystroke::Delete
            | Keystroke::Up
            | Keystroke::Down
            | Keystroke::WordLeft
            | Keystroke::WordRight
            | Keystroke::Home
            | Keystroke::End
            | Keystroke::ClearBefore
            | Keystroke::ClearAfter
            | Keystroke::DeleteWord
            | Keystroke::Escape(_)
            | Keystroke::Ignored => Step::Editing(self),
        }
    }

    fn view(&self, coloring: Coloring, _width: Option<usize>) -> Vec<String> {
        let option = |on: bool, label: &str| {
            let lit = format!("{} {label}", coloring.green("●"));
            pick(on, [coloring.dim(&format!("○ {label}")), lit])
        };
        vec![format!(
            "{}  {} {} {}",
            coloring.cyan("│"),
            option(self.yes, "Yes"),
            coloring.dim("/"),
            option(!self.yes, "No")
        )]
    }

    fn summary(&self) -> String {
        pick(self.yes, ["No", "Yes"]).to_owned()
    }
}

fn pick<T: Default>(on: bool, options: [T; 2]) -> T {
    options.into_iter().nth(usize::from(on)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{Confirm, Prompt, Step};
    use crate::{
        form::line::{Cancellation, Keystroke},
        output::coloring::Coloring,
    };

    #[derive(Debug, PartialEq, Eq)]
    enum End {
        Editing(bool),
        Submit(bool),
        Quit(Cancellation),
    }

    fn run(default: bool, keys: &[Keystroke]) -> End {
        let last = keys.iter().try_fold(Confirm::new(default), |confirm, key| {
            match confirm.press(*key) {
                Step::Editing(confirm) => Ok(confirm),
                Step::Submit(answer) => Err(End::Submit(answer)),
                Step::Quit(quit) => Err(End::Quit(quit)),
            }
        });
        last.map_or_else(|end| end, |confirm| End::Editing(confirm.yes))
    }

    #[rstest]
    #[case::no_by_default(false, &[Keystroke::Enter], End::Submit(false))]
    #[case::yes_by_default(true, &[Keystroke::Enter], End::Submit(true))]
    #[case::y_submits(false, &[Keystroke::Char('y')], End::Submit(true))]
    #[case::capital_y(false, &[Keystroke::Char('Y')], End::Submit(true))]
    #[case::n_submits(true, &[Keystroke::Char('n')], End::Submit(false))]
    #[case::capital_n(true, &[Keystroke::Char('N')], End::Submit(false))]
    #[case::left_flips(false, &[Keystroke::Left], End::Editing(true))]
    #[case::right_flips(true, &[Keystroke::Right], End::Editing(false))]
    #[case::tab_flips(false, &[Keystroke::Tab], End::Editing(true))]
    #[case::h_flips(false, &[Keystroke::Char('h')], End::Editing(true))]
    #[case::l_flips(true, &[Keystroke::Char('l')], End::Editing(false))]
    #[case::flip_twice(false, &[Keystroke::Tab, Keystroke::Tab], End::Editing(false))]
    #[case::enter_takes_the_flipped(false, &[Keystroke::Right, Keystroke::Enter], End::Submit(true))]
    #[case::other_keys_do_nothing(
        false,
        &[Keystroke::Char('x'), Keystroke::Up, Keystroke::Down, Keystroke::Backspace, Keystroke::Ignored],
        End::Editing(false)
    )]
    #[case::ctrl_c(false, &[Keystroke::Interrupt], End::Quit(Cancellation::Interrupt))]
    #[case::escape(false, &[Keystroke::Cancel], End::Quit(Cancellation::Cancel))]
    #[case::ctrl_d(true, &[Keystroke::EndOfInput], End::Quit(Cancellation::Cancel))]
    fn keys_move_or_end_the_question(
        #[case] default: bool,
        #[case] keys: &[Keystroke],
        #[case] end: End,
    ) {
        assert_eq!(run(default, keys), end);
    }

    #[rstest]
    #[case::no(false, "│  ○ Yes / ● No", "No")]
    #[case::yes(true, "│  ● Yes / ○ No", "Yes")]
    fn view_marks_the_highlighted_answer(
        #[case] default: bool,
        #[case] row: &str,
        #[case] summary: &str,
    ) {
        let confirm = Confirm::new(default);
        assert_eq!(confirm.view(Coloring::Plain, Some(80)), [row]);
        assert_eq!(confirm.summary(), summary);
    }
}
