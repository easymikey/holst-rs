//! Prompts as pure machines: a key gives the next [`Step`]; no IO, no clock.

use crate::{
    coloring::Coloring,
    line::{Cancellation, Keystroke},
};

/// What a key did to a prompt `M` whose answer is `A`.
#[derive(Debug)]
pub enum Step<M, A> {
    Editing(M),
    Submit(A),
    Quit(Cancellation),
}

pub trait Prompt: Sized {
    type Answer;

    #[must_use]
    fn press(self, key: Keystroke) -> Step<Self, Self::Answer>;

    /// Rows under the label while the prompt is active; no escapes for moves.
    #[must_use]
    fn view(&self, coloring: Coloring, width: Option<usize>) -> Vec<String>;

    /// The text of the line that stays once the prompt is submitted.
    #[must_use]
    fn summary(&self) -> String;

    /// Rows a keyless terminal shows under the label; none for a yes or no.
    #[must_use]
    fn listing(&self) -> Vec<String>;

    /// The keys a typed line stands for on a terminal without keys.
    #[must_use]
    fn typed(&self, line: &str) -> Vec<Keystroke>;
}

/// Yes or No; Enter takes the highlighted one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirm {
    yes: bool,
}

impl Confirm {
    #[must_use]
    pub const fn new(default: bool) -> Self {
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

    fn listing(&self) -> Vec<String> {
        Vec::new()
    }

    /// The first letter answers; anything else takes the highlighted one.
    fn typed(&self, line: &str) -> Vec<Keystroke> {
        line.trim()
            .chars()
            .take(1)
            .map(Keystroke::Char)
            .chain([Keystroke::Enter])
            .collect()
    }
}

/// One option of a [`Select`]: what it is called, and what tells it apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub hint: Option<String>,
}

impl Choice {
    fn hinted(&self, wrap: impl Fn(&str) -> String) -> String {
        self.hint
            .iter()
            .map(|hint| wrap(&format!(" ({hint})")))
            .collect()
    }
}

/// One option out of several; the answer is its place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Select {
    options: Vec<Choice>,
    cursor: usize,
    rows: usize,
}

impl Select {
    /// `None` without options; at most `rows` of them show at once.
    #[must_use]
    pub fn new(options: Vec<Choice>, rows: usize) -> Option<Self> {
        (!options.is_empty()).then_some(Self {
            options,
            cursor: 0,
            rows: rows.max(1),
        })
    }

    fn moved(self, step: usize) -> Self {
        let cursor = wrapped(self.cursor, step, self.options.len());
        Self { cursor, ..self }
    }
}

/// The cursor `step` options on, round the ends.
fn wrapped(cursor: usize, step: usize, count: usize) -> usize {
    cursor.saturating_add(step).checked_rem(count).unwrap_or(0)
}

/// The rows that show: at most `rows`, the cursor inside, `…` where more hide.
fn scrolled(
    coloring: Coloring,
    options: &[Choice],
    (cursor, rows): (usize, usize),
    row: impl Fn(usize, &Choice) -> String,
) -> Vec<String> {
    let start = cursor
        .saturating_sub(rows.div_ceil(2))
        .min(options.len().saturating_sub(rows));
    let end = start.saturating_add(rows).min(options.len());
    let more = |hidden: bool| {
        hidden
            .then(|| format!("{}  {}", coloring.cyan("│"), coloring.dim("…")))
            .into_iter()
    };
    let shown = options
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(at, option)| row(at, option));
    more(start > 0)
        .chain(shown)
        .chain(more(end < options.len()))
        .collect()
}

impl Prompt for Select {
    type Answer = usize;

    fn press(self, key: Keystroke) -> Step<Self, usize> {
        match key {
            Keystroke::Up | Keystroke::Char('k') => {
                let back = self.options.len().saturating_sub(1);
                Step::Editing(self.moved(back))
            }
            Keystroke::Down | Keystroke::Tab | Keystroke::Char('j') => {
                Step::Editing(self.moved(1))
            }
            Keystroke::Enter => Step::Submit(self.cursor),
            Keystroke::Interrupt => Step::Quit(Cancellation::Interrupt),
            Keystroke::Cancel | Keystroke::EndOfInput => {
                Step::Quit(Cancellation::Cancel)
            }
            Keystroke::Char(_)
            | Keystroke::Left
            | Keystroke::Right
            | Keystroke::Backspace
            | Keystroke::Delete
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
        let bar = coloring.cyan("│");
        scrolled(
            coloring,
            &self.options,
            (self.cursor, self.rows),
            |at, option| {
                let active = format!(
                    "{bar}  {} {}{}",
                    coloring.green("●"),
                    option.label,
                    option.hinted(|hint| coloring.dim(hint))
                );
                let idle = format!(
                    "{bar}  {}",
                    coloring.dim(&format!(
                        "○ {}{}",
                        option.label,
                        option.hinted(str::to_owned)
                    ))
                );
                pick(at == self.cursor, [idle, active])
            },
        )
    }

    fn summary(&self) -> String {
        self.options
            .get(self.cursor)
            .map(|option| option.label.clone())
            .unwrap_or_default()
    }

    fn listing(&self) -> Vec<String> {
        self.options
            .iter()
            .zip(1_usize..)
            .map(|(option, number)| {
                format!(
                    "  {number}) {}{}",
                    option.label,
                    option.hinted(str::to_owned)
                )
            })
            .collect()
    }

    /// A number picks that option; anything else cancels.
    fn typed(&self, line: &str) -> Vec<Keystroke> {
        match line.trim().parse::<usize>() {
            Ok(number) if (1..=self.options.len()).contains(&number) => {
                std::iter::repeat_n(Keystroke::Down, number.saturating_sub(1))
                    .chain([Keystroke::Enter])
                    .collect()
            }
            Ok(_) | Err(_) => vec![Keystroke::Cancel],
        }
    }
}

/// Any options out of several; the answer is their places in option order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiSelect {
    options: Vec<Choice>,
    cursor: usize,
    picked: Vec<bool>,
    required: bool,
    warned: bool,
    rows: usize,
}

impl MultiSelect {
    /// `None` without options; a short `picked` leaves the rest unpicked.
    #[must_use]
    pub(crate) fn new(
        options: Vec<Choice>,
        picked: Vec<bool>,
        required: bool,
        rows: usize,
    ) -> Option<Self> {
        let picked = picked
            .into_iter()
            .chain(std::iter::repeat(false))
            .take(options.len())
            .collect();
        (!options.is_empty()).then_some(Self {
            options,
            cursor: 0,
            picked,
            required,
            warned: false,
            rows: rows.max(1),
        })
    }

    fn moved(self, step: usize) -> Self {
        let cursor = wrapped(self.cursor, step, self.options.len());
        Self { cursor, ..self }
    }

    fn toggled(self) -> Self {
        let picked = (0..)
            .zip(&self.picked)
            .map(|(at, on)| *on != (at == self.cursor))
            .collect();
        Self { picked, ..self }
    }

    /// Everything picked, or nothing when everything already is.
    fn all(self) -> Self {
        let picked = vec![!self.picked.iter().all(|on| *on); self.picked.len()];
        Self { picked, ..self }
    }

    fn inverted(self) -> Self {
        let picked = self.picked.iter().map(|on| !on).collect();
        Self { picked, ..self }
    }

    fn places(&self) -> Vec<usize> {
        (0..)
            .zip(&self.picked)
            .filter_map(|(at, on)| on.then_some(at))
            .collect()
    }

    /// The labels of the options at `places`, or `none`.
    pub(crate) fn labelled(&self, places: &[usize]) -> String {
        let labels: Vec<&str> = places
            .iter()
            .filter_map(|at| self.options.get(*at))
            .map(|option| option.label.as_str())
            .collect();
        match labels.as_slice() {
            [] => "none".to_owned(),
            some => some.join(", "),
        }
    }
}

impl Prompt for MultiSelect {
    type Answer = Vec<usize>;

    fn press(self, key: Keystroke) -> Step<Self, Vec<usize>> {
        let calm = Self {
            warned: false,
            ..self
        };
        match key {
            Keystroke::Up | Keystroke::Char('k') => {
                let back = calm.options.len().saturating_sub(1);
                Step::Editing(calm.moved(back))
            }
            Keystroke::Down | Keystroke::Tab | Keystroke::Char('j') => {
                Step::Editing(calm.moved(1))
            }
            Keystroke::Char(' ') => Step::Editing(calm.toggled()),
            Keystroke::Char('a') => Step::Editing(calm.all()),
            Keystroke::Char('i') => Step::Editing(calm.inverted()),
            Keystroke::Enter if calm.required && calm.places().is_empty() => {
                Step::Editing(Self {
                    warned: true,
                    ..calm
                })
            }
            Keystroke::Enter => Step::Submit(calm.places()),
            Keystroke::Interrupt => Step::Quit(Cancellation::Interrupt),
            Keystroke::Cancel | Keystroke::EndOfInput => {
                Step::Quit(Cancellation::Cancel)
            }
            Keystroke::Char(_)
            | Keystroke::Left
            | Keystroke::Right
            | Keystroke::Backspace
            | Keystroke::Delete
            | Keystroke::WordLeft
            | Keystroke::WordRight
            | Keystroke::Home
            | Keystroke::End
            | Keystroke::ClearBefore
            | Keystroke::ClearAfter
            | Keystroke::DeleteWord
            | Keystroke::Escape(_)
            | Keystroke::Ignored => Step::Editing(calm),
        }
    }

    /// Picked rows show `◼`; under Plain the cursor row shows `›`.
    fn view(&self, coloring: Coloring, _width: Option<usize>) -> Vec<String> {
        let bar = coloring.cyan("│");
        let rows = scrolled(
            coloring,
            &self.options,
            (self.cursor, self.rows),
            |at, option| {
                let here = at == self.cursor;
                let lead = match (coloring, here) {
                    (Coloring::Plain, true) => " ›",
                    (Coloring::Plain | Coloring::Colored, _) => "  ",
                };
                let boxed = match (self.picked.get(at), here) {
                    (Some(true), _) => coloring.green("◼"),
                    (_, true) => coloring.cyan("◻"),
                    (_, false) => coloring.dim("◻"),
                };
                format!(
                    "{bar}{lead}{boxed} {}{}",
                    option.label,
                    option.hinted(|hint| coloring.dim(hint))
                )
            },
        );
        let warning = self.warned.then(|| {
            format!(
                "{}  {}",
                coloring.yellow("│"),
                coloring.yellow("▲ Please select at least one option.")
            )
        });
        rows.into_iter().chain(warning).collect()
    }

    fn summary(&self) -> String {
        self.labelled(&self.places())
    }

    fn listing(&self) -> Vec<String> {
        let rows = (1_usize..).zip(self.options.iter().zip(&self.picked)).map(
            |(number, (option, on))| {
                format!(
                    "  {number}) {} {}{}",
                    pick(*on, ["[ ]", "[x]"]),
                    option.label,
                    option.hinted(str::to_owned)
                )
            },
        );
        rows.chain(["  numbers to pick, e.g. 1 3; Enter keeps the marked".to_owned()])
            .collect()
    }

    /// Numbers name the options to pick; none keeps the marked; else cancels.
    fn typed(&self, line: &str) -> Vec<Keystroke> {
        let named: Option<Vec<usize>> = line
            .split([',', ' '])
            .filter(|word| !word.is_empty())
            .map(|word| {
                word.parse::<usize>()
                    .ok()
                    .filter(|number| (1..=self.options.len()).contains(number))
            })
            .collect();
        match named {
            None => vec![Keystroke::Cancel],
            Some(named) if named.is_empty() => vec![Keystroke::Enter],
            Some(named) => (1..)
                .zip(&self.picked)
                .flat_map(|(number, on)| {
                    (*on != named.contains(&number))
                        .then_some(Keystroke::Char(' '))
                        .into_iter()
                        .chain([Keystroke::Down])
                })
                .chain([Keystroke::Enter])
                .collect(),
        }
    }
}

fn pick<T: Default>(on: bool, options: [T; 2]) -> T {
    options.into_iter().nth(usize::from(on)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{Choice, Confirm, MultiSelect, Prompt, Select, Step};
    use crate::{
        coloring::Coloring,
        line::{Cancellation, Keystroke},
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

    #[derive(Debug, PartialEq, Eq)]
    enum Picked {
        Editing(usize),
        Submit(usize),
        Quit(Cancellation),
    }

    fn options(count: usize) -> Vec<Choice> {
        (1..=count)
            .map(|number| Choice {
                label: format!("Plans {number}"),
                hint: (number == 2).then(|| "id-2".to_owned()),
            })
            .collect()
    }

    fn pick(count: usize, keys: &[Keystroke]) -> Picked {
        let start = Select {
            options: options(count),
            cursor: 0,
            rows: 3,
        };
        keys.iter()
            .try_fold(start, |select, key| match select.press(*key) {
                Step::Editing(select) => Ok(select),
                Step::Submit(answer) => Err(Picked::Submit(answer)),
                Step::Quit(quit) => Err(Picked::Quit(quit)),
            })
            .map_or_else(|end| end, |select| Picked::Editing(select.cursor))
    }

    #[rstest]
    #[case::first_by_default(3, &[Keystroke::Enter], Picked::Submit(0))]
    #[case::down_then_enter(3, &[Keystroke::Down, Keystroke::Enter], Picked::Submit(1))]
    #[case::j_and_tab_go_down(3, &[Keystroke::Char('j'), Keystroke::Tab], Picked::Editing(2))]
    #[case::down_wraps(3, &[Keystroke::Down, Keystroke::Down, Keystroke::Down], Picked::Editing(0))]
    #[case::up_wraps(3, &[Keystroke::Up], Picked::Editing(2))]
    #[case::k_goes_up(3, &[Keystroke::Down, Keystroke::Char('k')], Picked::Editing(0))]
    #[case::one_option_stays(1, &[Keystroke::Down, Keystroke::Up], Picked::Editing(0))]
    #[case::other_keys_do_nothing(
        3,
        &[Keystroke::Char('x'), Keystroke::Left, Keystroke::Right, Keystroke::Backspace, Keystroke::Ignored],
        Picked::Editing(0)
    )]
    #[case::ctrl_c(3, &[Keystroke::Interrupt], Picked::Quit(Cancellation::Interrupt))]
    #[case::escape(3, &[Keystroke::Cancel], Picked::Quit(Cancellation::Cancel))]
    #[case::ctrl_d(3, &[Keystroke::EndOfInput], Picked::Quit(Cancellation::Cancel))]
    fn keys_move_the_cursor_or_end_the_choice(
        #[case] count: usize,
        #[case] keys: &[Keystroke],
        #[case] end: Picked,
    ) {
        assert_eq!(pick(count, keys), end);
    }

    #[test]
    fn no_options_make_no_select() {
        assert_eq!(Select::new(Vec::new(), 3), None);
    }

    #[rstest]
    #[case::top(0, &["│  ● Plans 1", "│  ○ Plans 2 (id-2)", "│  ○ Plans 3", "│  …"])]
    #[case::middle(3, &["│  …", "│  ○ Plans 2 (id-2)", "│  ○ Plans 3", "│  ● Plans 4", "│  …"])]
    #[case::bottom(4, &["│  …", "│  ○ Plans 3", "│  ○ Plans 4", "│  ● Plans 5"])]
    fn view_shows_a_window_round_the_cursor(
        #[case] cursor: usize,
        #[case] rows: &[&str],
    ) {
        let select = Select {
            options: options(5),
            cursor,
            rows: 3,
        };
        assert_eq!(select.view(Coloring::Plain, Some(80)), rows);
    }

    #[test]
    fn the_summary_is_the_label_of_the_option() {
        let select = Select {
            options: options(3),
            cursor: 1,
            rows: 3,
        };
        assert_eq!(select.summary(), "Plans 2");
    }

    #[rstest]
    #[case::second("2", &[Keystroke::Down, Keystroke::Enter])]
    #[case::first(" 1 ", &[Keystroke::Enter])]
    #[case::zero("0", &[Keystroke::Cancel])]
    #[case::past_the_end("4", &[Keystroke::Cancel])]
    #[case::words("two", &[Keystroke::Cancel])]
    #[case::empty("", &[Keystroke::Cancel])]
    fn a_typed_number_picks_that_option(
        #[case] line: &str,
        #[case] keys: &[Keystroke],
    ) {
        let select = Select {
            options: options(3),
            cursor: 0,
            rows: 3,
        };
        assert_eq!(select.typed(line), keys);
        assert_eq!(
            select.listing(),
            ["  1) Plans 1", "  2) Plans 2 (id-2)", "  3) Plans 3"]
        );
    }

    fn multi(count: usize, picked: &[bool], required: bool) -> MultiSelect {
        MultiSelect {
            options: options(count),
            cursor: 0,
            picked: picked.to_vec(),
            required,
            warned: false,
            rows: 3,
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Marked {
        Editing {
            places: Vec<usize>,
            cursor: usize,
            warned: bool,
        },
        Submit(Vec<usize>),
        Quit(Cancellation),
    }

    fn mark(start: MultiSelect, keys: &[Keystroke]) -> Marked {
        keys.iter()
            .try_fold(start, |select, key| match select.press(*key) {
                Step::Editing(select) => Ok(select),
                Step::Submit(places) => Err(Marked::Submit(places)),
                Step::Quit(quit) => Err(Marked::Quit(quit)),
            })
            .map_or_else(
                |end| end,
                |select| Marked::Editing {
                    places: select.places(),
                    cursor: select.cursor,
                    warned: select.warned,
                },
            )
    }

    fn editing(places: &[usize], cursor: usize, warned: bool) -> Marked {
        Marked::Editing {
            places: places.to_vec(),
            cursor,
            warned,
        }
    }

    const SPACE: Keystroke = Keystroke::Char(' ');
    const NONE: [bool; 3] = [false; 3];

    #[rstest]
    #[case::space_picks(NONE, false, &[SPACE], editing(&[0], 0, false))]
    #[case::space_twice_unpicks(NONE, false, &[SPACE, SPACE], editing(&[], 0, false))]
    #[case::space_follows_the_cursor(NONE, false, &[Keystroke::Down, SPACE], editing(&[1], 1, false))]
    #[case::enter_answers_in_option_order(
        NONE,
        false,
        &[Keystroke::Down, Keystroke::Down, SPACE, Keystroke::Up, Keystroke::Up, SPACE, Keystroke::Enter],
        Marked::Submit(vec![0, 2])
    )]
    #[case::start_picked_is_kept([false, true, true], false, &[Keystroke::Enter], Marked::Submit(vec![1, 2]))]
    #[case::a_picks_all(NONE, false, &[Keystroke::Char('a')], editing(&[0, 1, 2], 0, false))]
    #[case::a_picks_the_rest([true, false, false], false, &[Keystroke::Char('a')], editing(&[0, 1, 2], 0, false))]
    #[case::a_twice_clears(NONE, false, &[Keystroke::Char('a'), Keystroke::Char('a')], editing(&[], 0, false))]
    #[case::i_inverts([true, false, false], false, &[Keystroke::Char('i')], editing(&[1, 2], 0, false))]
    #[case::i_twice_restores([true, false, false], false, &[Keystroke::Char('i'), Keystroke::Char('i')], editing(&[0], 0, false))]
    #[case::nothing_answers_when_not_required(NONE, false, &[Keystroke::Enter], Marked::Submit(vec![]))]
    #[case::required_warns_instead_of_answering(NONE, true, &[Keystroke::Enter], editing(&[], 0, true))]
    #[case::the_next_key_clears_the_warning(NONE, true, &[Keystroke::Enter, Keystroke::Down], editing(&[], 1, false))]
    #[case::warning_comes_again(NONE, true, &[Keystroke::Enter, Keystroke::Down, Keystroke::Enter], editing(&[], 1, true))]
    #[case::pick_after_warning_answers(NONE, true, &[Keystroke::Enter, SPACE, Keystroke::Enter], Marked::Submit(vec![0]))]
    #[case::required_with_a_pick_answers([true, false, false], true, &[Keystroke::Enter], Marked::Submit(vec![0]))]
    #[case::j_tab_and_down_go_down(NONE, false, &[Keystroke::Char('j'), Keystroke::Tab], editing(&[], 2, false))]
    #[case::down_wraps(NONE, false, &[Keystroke::Down, Keystroke::Down, Keystroke::Down], editing(&[], 0, false))]
    #[case::up_wraps(NONE, false, &[Keystroke::Up], editing(&[], 2, false))]
    #[case::k_goes_up(NONE, false, &[Keystroke::Down, Keystroke::Char('k')], editing(&[], 0, false))]
    #[case::other_keys_do_nothing(
        NONE,
        false,
        &[Keystroke::Char('x'), Keystroke::Left, Keystroke::Right, Keystroke::Backspace, Keystroke::Ignored],
        editing(&[], 0, false)
    )]
    #[case::ctrl_c(NONE, false, &[Keystroke::Interrupt], Marked::Quit(Cancellation::Interrupt))]
    #[case::escape(NONE, false, &[Keystroke::Cancel], Marked::Quit(Cancellation::Cancel))]
    #[case::ctrl_d(NONE, false, &[Keystroke::EndOfInput], Marked::Quit(Cancellation::Cancel))]
    fn keys_pick_or_end_the_multiselect(
        #[case] picked: [bool; 3],
        #[case] required: bool,
        #[case] keys: &[Keystroke],
        #[case] end: Marked,
    ) {
        assert_eq!(mark(multi(3, &picked, required), keys), end);
    }

    #[test]
    fn no_options_make_no_multiselect() {
        assert_eq!(MultiSelect::new(Vec::new(), Vec::new(), false, 3), None);
    }

    #[test]
    fn a_short_picked_list_leaves_the_rest_unpicked() {
        let select = MultiSelect::new(options(3), vec![true], false, 3);
        assert_eq!(select.map(|select| select.places()), Some(vec![0]));
    }

    #[rstest]
    #[case::cursor_marker_and_boxes(
        3,
        1,
        &[true, false, false],
        false,
        &["│  ◼ Plans 1", "│ ›◻ Plans 2 (id-2)", "│  ◻ Plans 3"]
    )]
    #[case::picked_under_the_cursor(
        3,
        0,
        &[true, false, true],
        false,
        &["│ ›◼ Plans 1", "│  ◻ Plans 2 (id-2)", "│  ◼ Plans 3"]
    )]
    #[case::warning_row(
        3,
        0,
        &[false, false, false],
        true,
        &["│ ›◻ Plans 1", "│  ◻ Plans 2 (id-2)", "│  ◻ Plans 3", "│  ▲ Please select at least one option."]
    )]
    #[case::top(5, 0, &[false; 5], false, &["│ ›◻ Plans 1", "│  ◻ Plans 2 (id-2)", "│  ◻ Plans 3", "│  …"])]
    #[case::middle(5, 3, &[false; 5], false, &["│  …", "│  ◻ Plans 2 (id-2)", "│  ◻ Plans 3", "│ ›◻ Plans 4", "│  …"])]
    #[case::bottom(5, 4, &[false, false, false, false, true], false, &["│  …", "│  ◻ Plans 3", "│  ◻ Plans 4", "│ ›◼ Plans 5"])]
    fn view_marks_the_cursor_and_the_picked_rows(
        #[case] count: usize,
        #[case] cursor: usize,
        #[case] picked: &[bool],
        #[case] warned: bool,
        #[case] rows: &[&str],
    ) {
        let select = MultiSelect {
            cursor,
            warned,
            ..multi(count, picked, true)
        };
        assert_eq!(select.view(Coloring::Plain, Some(80)), rows);
    }

    #[rstest]
    #[case::none(&[false, false, false], "none")]
    #[case::one(&[false, true, false], "Plans 2")]
    #[case::several(&[true, false, true], "Plans 1, Plans 3")]
    fn the_summary_names_the_picked_options(
        #[case] picked: &[bool],
        #[case] summary: &str,
    ) {
        assert_eq!(multi(3, picked, false).summary(), summary);
    }

    #[rstest]
    #[case::keep_the_marked("", &[Keystroke::Enter])]
    #[case::blanks("  ", &[Keystroke::Enter])]
    #[case::third_only(
        "3",
        &[Keystroke::Down, SPACE, Keystroke::Down, SPACE, Keystroke::Down, Keystroke::Enter]
    )]
    #[case::first_and_third(
        "1, 3",
        &[SPACE, Keystroke::Down, SPACE, Keystroke::Down, SPACE, Keystroke::Down, Keystroke::Enter]
    )]
    #[case::commas("1,2", &[SPACE, Keystroke::Down, Keystroke::Down, Keystroke::Down, Keystroke::Enter])]
    #[case::zero("0", &[Keystroke::Cancel])]
    #[case::past_the_end("1 4", &[Keystroke::Cancel])]
    #[case::words("all", &[Keystroke::Cancel])]
    fn typed_numbers_name_the_picked_options(
        #[case] line: &str,
        #[case] keys: &[Keystroke],
    ) {
        let select = multi(3, &[false, true, false], false);
        assert_eq!(select.typed(line), keys);
    }

    #[test]
    fn the_listing_marks_the_picked_options() {
        assert_eq!(
            multi(3, &[false, true, false], false).listing(),
            [
                "  1) [ ] Plans 1",
                "  2) [x] Plans 2 (id-2)",
                "  3) [ ] Plans 3",
                "  numbers to pick, e.g. 1 3; Enter keeps the marked"
            ]
        );
    }
}
