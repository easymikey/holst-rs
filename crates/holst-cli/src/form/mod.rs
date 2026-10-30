//! The login form à la `@clack/prompts`; a secret shows only as `▪`.

pub(crate) mod draw;
#[cfg(test)] pub(crate) mod fake;
pub(crate) mod line;
pub(crate) mod prompt;
pub(crate) mod terminal;

use std::{convert::Infallible, ops::ControlFlow, time::Duration};

use zeroize::Zeroizing;

use self::{
    draw::{NoteTone, QuestionFooter},
    line::{Cancellation, Decoder, EditorStep, Keystroke, QuestionKind, TypedAnswer},
    prompt::{Confirm, Prompt, Step},
    terminal::{Terminal, TerminalKind},
};
use crate::{
    error::{Error, InvalidAnswer, Result},
    output::{coloring::Coloring, view},
    terminal::signals,
};

const SPIN_STEP: Duration = Duration::from_millis(80);

enum KeyReadEnd {
    Typed(Zeroizing<String>),
    GaveUp {
        masked: String,
        cancellation: Cancellation,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Question {
    pub(crate) label: &'static str,
    pub(crate) kind: QuestionKind,
}

#[derive(Debug)]
pub(crate) struct Form<T: Terminal> {
    terminal: T,
    coloring: Coloring,
    _quiet: T::QuietTerminal,
}

impl<T: Terminal> Form<T> {
    /// A form without an intro line: one question inside a command.
    pub(crate) fn bare(term: T) -> Self {
        Self {
            _quiet: term.quiet(),
            coloring: Coloring::colored_if(term.has_color()),
            terminal: term,
        }
    }

    pub(crate) fn intro(term: T, title: &str) -> Result<Self> {
        let form = Self::bare(term);
        form.write_live(&draw::clear_line())?;
        form.write(&draw::intro(form.coloring, title))
            .map(|()| form)
    }

    fn not_interrupted(&self) -> Result<()> {
        if self.terminal.interrupts() == 0 {
            Ok(())
        } else {
            Err(Error::Interrupted)
        }
    }

    #[must_use]
    pub(crate) const fn terminal(&self) -> &T {
        &self.terminal
    }

    pub(crate) fn ask<V>(
        &self,
        label: &'static str,
        kind: QuestionKind,
        parse: &impl Fn(&str) -> Result<V, InvalidAnswer>,
    ) -> Result<V> {
        self.ask_with(Question { label, kind }, parse, QuestionFooter::Plain)
    }

    pub(crate) fn ask_again<V>(
        &self,
        question: Question,
        parse: &impl Fn(&str) -> Result<V, InvalidAnswer>,
        invalid: InvalidAnswer,
    ) -> Result<V> {
        match self.terminal.kind() {
            TerminalKind::Live | TerminalKind::Dumb => {
                self.ask_with(question, parse, QuestionFooter::Warn(invalid))
            }
            TerminalKind::Redirected => Err(Error::InvalidAnswer {
                question: question.label,
                reason: invalid,
            }),
        }
    }

    fn ask_with<V>(
        &self,
        question: Question,
        parse: &impl Fn(&str) -> Result<V, InvalidAnswer>,
        footer: QuestionFooter,
    ) -> Result<V> {
        let Question { label, kind } = question;
        let typed = self.read(label, kind, footer)?;
        match parse(&typed) {
            Ok(value) => self
                .write(&draw::answered(
                    self.coloring,
                    label,
                    &draw::masked(kind, &typed),
                ))
                .map(|()| value),
            Err(invalid) => self.ask_again(question, parse, invalid),
        }
    }

    /// Asks yes or no; Enter takes `default`.
    pub(crate) fn confirm(&self, label: &str, default: bool) -> Result<bool> {
        let yes = self.run(label, Confirm::new(default))?;
        self.step(label, &Confirm::new(yes).summary()).map(|()| yes)
    }

    fn run<P: Prompt>(&self, label: &str, prompt: P) -> Result<P::Answer> {
        self.not_interrupted()?;
        match self.terminal.kind() {
            TerminalKind::Live => {
                self.terminal.answering(|_| self.run_keys(label, prompt))
            }
            TerminalKind::Dumb => {
                self.terminal.answering(|_| self.run_lines(label, prompt))
            }
            TerminalKind::Redirected => Err(Error::NoInput { what: "an answer" }),
        }
    }

    fn run_keys<P: Prompt>(&self, label: &str, prompt: P) -> Result<P::Answer> {
        let done = self
            .terminal
            .hidden(|_| self.press_keys(label, prompt))
            .map_err(Error::Terminal)?;
        self.terminal.discard_typeahead();
        done
    }

    fn press_keys<P: Prompt>(&self, label: &str, prompt: P) -> Result<P::Answer> {
        let frame = |shown: &P| {
            std::iter::once(format!("{}  {label}", self.coloring.cyan("◆")))
                .chain(shown.view(self.coloring, self.terminal.width()))
                .chain([self.coloring.cyan("└")])
                .collect::<Vec<_>>()
        };
        let first = frame(&prompt);
        self.write(&draw::repaint(0, &first))?;
        let flow = std::iter::repeat_with(|| self.terminal.keystroke()).try_fold(
            (prompt, Decoder::default(), first.len()),
            |(prompt, decoder, drawn), keystroke| match keystroke {
                Err(error) => ControlFlow::Break(Err(Error::Terminal(error))),
                Ok(keystroke) => {
                    let (decoder, key) = decoder.feed(keystroke);
                    match prompt.press(key) {
                        Step::Editing(prompt) => {
                            let rows = frame(&prompt);
                            self.write(&draw::repaint(drawn, &rows)).map_or_else(
                                |error| ControlFlow::Break(Err(error)),
                                |()| {
                                    ControlFlow::Continue((prompt, decoder, rows.len()))
                                },
                            )
                        }
                        Step::Submit(answer) => ControlFlow::Break(
                            self.write(&draw::repaint(drawn, &[])).map(|()| answer),
                        ),
                        Step::Quit(quit) => ControlFlow::Break(
                            self.write(&draw::repaint(drawn, &[]))
                                .and_then(|()| {
                                    self.write(&draw::cancelled(
                                        self.coloring,
                                        label,
                                        "",
                                    ))
                                })
                                .and(Err(stopped_by(quit))),
                        ),
                    }
                }
            },
        );
        match flow {
            ControlFlow::Break(done) => done,
            // Keystrokes never run out; if they did, nothing was submitted.
            ControlFlow::Continue(_) => Err(Error::Cancelled),
        }
    }

    /// A line answers with its first letter; anything else takes the default.
    fn run_lines<P: Prompt>(&self, label: &str, prompt: P) -> Result<P::Answer> {
        let line = self
            .terminal
            .write(&format!("◆  {label}\n"))
            .and_then(|()| self.terminal.line())
            .inspect(|_| self.terminal.discard_typeahead())
            .map_err(Error::ReadInput)?
            .ok_or(Error::Cancelled)?;
        let flow = line
            .trim()
            .chars()
            .take(1)
            .map(Keystroke::Char)
            .chain([Keystroke::Enter])
            .try_fold(prompt, |prompt, key| match prompt.press(key) {
                Step::Editing(prompt) => ControlFlow::Continue(prompt),
                Step::Submit(answer) => ControlFlow::Break(Ok(answer)),
                Step::Quit(quit) => ControlFlow::Break(Err(stopped_by(quit))),
            });
        match flow {
            ControlFlow::Break(done) => done,
            ControlFlow::Continue(_) => Err(Error::Cancelled),
        }
    }

    pub(crate) fn step(&self, label: &str, value: &str) -> Result<()> {
        self.write(&draw::answered(self.coloring, label, value))
    }

    pub(crate) fn write_box(
        &self,
        title: &str,
        lines: &[(NoteTone, String)],
    ) -> Result<()> {
        self.write(&draw::boxed(
            self.coloring,
            title,
            lines,
            self.terminal.width(),
        ))
    }

    pub(crate) async fn wait<V, E>(
        &self,
        label: &str,
        work: impl Future<Output = Result<V, E>>,
    ) -> Result<V>
    where
        Error: From<E>,
    {
        // Listen before the line shows, so an early Ctrl-C reaches this wait.
        let interrupt = self.terminal.interrupt();
        self.not_interrupted()?;
        self.write_live(&draw::waiting(self.coloring, label))?;
        let out = signals::or_interrupted(
            async {
                tokio::select! {
                    out = work => out.map_err(Error::from),
                    never = self.spin(label) => match never {},
                }
            },
            interrupt,
            || Err(Error::Interrupted),
        )
        .await;
        self.write_live(&draw::done_waiting())?;
        out
    }

    async fn spin(&self, label: &str) -> Infallible {
        match self.terminal.kind() {
            TerminalKind::Live => self.turn(label).await,
            TerminalKind::Dumb | TerminalKind::Redirected => {
                std::future::pending().await
            }
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the spinner paces frames on the tokio timer; there is no clock in the form"
    )]
    async fn turn(&self, label: &str) -> Infallible {
        let origin = tokio::time::Instant::now();
        let frames = std::cell::Cell::new(1_u32);
        // imperative: the frames repeat until the request ends and `select!` drops this
        loop {
            let frame = frames.get();
            tokio::time::sleep_until(origin + SPIN_STEP.saturating_mul(frame)).await;
            frames.set(frame.saturating_add(1));
            self.write(&draw::spinning(
                self.coloring,
                label,
                usize::try_from(frame).unwrap_or_default(),
            ))
            .unwrap_or(());
        }
    }

    pub(crate) fn outro(&self, message: &str) -> Result<()> {
        self.write(&draw::outro(self.coloring, message))
    }

    pub(crate) fn cancel(&self, error: &Error) -> Result<()> {
        let (message, causes) =
            if matches!(error, Error::Interrupted | Error::Cancelled) {
                (Error::Cancelled.to_string(), Vec::new())
            } else {
                (
                    holst_core::domain::text::printable(&error.to_string())
                        .into_owned(),
                    view::form_cause_lines(error, self.coloring),
                )
            };
        self.write(&draw::error_outro(self.coloring, &message, &causes))
    }

    fn read(
        &self,
        label: &'static str,
        kind: QuestionKind,
        footer: QuestionFooter,
    ) -> Result<Zeroizing<String>> {
        self.not_interrupted()?;
        self.terminal.answering(|term| match term.kind() {
            TerminalKind::Live => self.read_live(label, kind, footer),
            TerminalKind::Dumb => self.read_blind(label, kind, footer),
            TerminalKind::Redirected => self.read_piped(label, kind),
        })
    }

    fn read_live(
        &self,
        label: &str,
        kind: QuestionKind,
        footer: QuestionFooter,
    ) -> Result<Zeroizing<String>> {
        // Show the question only once the echo is off.
        let ask = || {
            let first = draw::draw_input(
                self.coloring,
                &TypedAnswer::new(kind),
                self.terminal.width(),
            );
            self.write(&draw::active(self.coloring, label, &first, footer))?;
            let typed = self.read_keys(TypedAnswer::new(kind));
            // Drop the unread rest of a paste while the echo is still off.
            self.terminal.discard_typeahead();
            typed
        };
        let typed = self.terminal.hidden(|_| ask()).unwrap_or_else(|_| ask());
        self.write(&draw::erase_active())?;
        match typed? {
            KeyReadEnd::Typed(text) => Ok(text),
            KeyReadEnd::GaveUp {
                masked: shown,
                cancellation: quit,
            } => self
                .write(&draw::cancelled(self.coloring, label, &shown))
                .and(Err(stopped_by(quit))),
        }
    }

    fn read_keys(&self, answer: TypedAnswer) -> Result<KeyReadEnd> {
        let flow = std::iter::repeat_with(|| self.terminal.keystroke()).try_fold(
            (answer, Decoder::default()),
            |(answer, decoder), keystroke| {
                let shown = draw::masked(answer.kind(), answer.text());
                match keystroke.map_err(Error::Terminal).map(|keystroke| {
                    let (decoder, key) = decoder.feed(keystroke);
                    (decoder, answer.press(key))
                }) {
                    Ok((decoder, EditorStep::Editing(answer))) => {
                        let redrawn = draw::draw_input(
                            self.coloring,
                            &answer,
                            self.terminal.width(),
                        );
                        self.write(&draw::update(self.coloring, &redrawn))
                            .map_or_else(
                                |error| ControlFlow::Break(Err(error)),
                                |()| ControlFlow::Continue((answer, decoder)),
                            )
                    }
                    Ok((_, EditorStep::Submit(text))) => {
                        ControlFlow::Break(Ok(KeyReadEnd::Typed(text)))
                    }
                    Ok((_, EditorStep::Quit(quit))) => {
                        ControlFlow::Break(Ok(KeyReadEnd::GaveUp {
                            masked: shown,
                            cancellation: quit,
                        }))
                    }
                    Err(error) => ControlFlow::Break(Err(error)),
                }
            },
        );
        match flow {
            ControlFlow::Break(done) => done,
            // Keystrokes never run out; if they did, nothing was submitted.
            ControlFlow::Continue(_) => Err(Error::Cancelled),
        }
    }

    fn read_blind(
        &self,
        label: &str,
        kind: QuestionKind,
        footer: QuestionFooter,
    ) -> Result<Zeroizing<String>> {
        let warning = match footer {
            QuestionFooter::Plain => String::new(),
            QuestionFooter::Warn(invalid) => format!("▲  {invalid}\n"),
        };
        let question = format!("{warning}◆  {label}\n");
        let line = match kind {
            QuestionKind::LoginCode | QuestionKind::Password => self
                .terminal
                .hidden(|term| {
                    term.write(&question)
                        .and_then(|()| term.line())
                        .inspect(|_| term.discard_typeahead())
                })
                .map_err(Error::EchoNotHidden)?,
            QuestionKind::Text => self
                .terminal
                .write(&question)
                .and_then(|()| self.terminal.line())
                .inspect(|_| self.terminal.discard_typeahead()),
        };
        line.map_err(Error::ReadInput)?
            .map(|text| clean_answer(kind, &text))
            .ok_or(Error::Cancelled)
    }

    fn read_piped(
        &self,
        label: &'static str,
        kind: QuestionKind,
    ) -> Result<Zeroizing<String>> {
        self.terminal
            .line()
            .map_err(Error::ReadInput)?
            .map(|text| clean_answer(kind, &Zeroizing::new(text)))
            .ok_or(Error::NoInput { what: label })
    }

    fn write_live(&self, text: &str) -> Result<()> {
        match self.terminal.kind() {
            TerminalKind::Live => self.write(text),
            TerminalKind::Dumb | TerminalKind::Redirected => Ok(()),
        }
    }

    fn write(&self, text: &str) -> Result<()> {
        self.terminal.write(text).map_err(Error::Terminal)
    }
}

fn stopped_by(quit: Cancellation) -> Error {
    match quit {
        Cancellation::Interrupt => Error::Interrupted,
        Cancellation::Cancel => Error::Cancelled,
    }
}

fn clean_answer(kind: QuestionKind, text: &str) -> Zeroizing<String> {
    let done = text
        .chars()
        .map(Keystroke::Char)
        .chain([Keystroke::Enter])
        .try_fold(TypedAnswer::new(kind), |answer, keystroke| {
            match answer.press(keystroke) {
                EditorStep::Editing(answer) => ControlFlow::Continue(answer),
                EditorStep::Submit(submitted) => ControlFlow::Break(submitted),
                EditorStep::Quit(_) => {
                    ControlFlow::Break(Zeroizing::new(String::new()))
                }
            }
        });
    match done {
        ControlFlow::Break(cleaned) => cleaned,
        ControlFlow::Continue(_) => Zeroizing::new(String::new()),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use rstest::rstest;

    use crate::{
        error::{Error, InvalidAnswer, Result},
        form::{
            Form,
            Question,
            fake::{Script, screen, without_escapes},
            line::{Keystroke, QuestionKind},
            terminal::TerminalKind,
        },
    };

    fn chars(text: &'static str) -> impl Iterator<Item = Keystroke> {
        text.chars().map(Keystroke::Char)
    }

    fn not_empty(text: &str) -> Result<String, InvalidAnswer> {
        if text.is_empty() {
            Err(InvalidAnswer::Empty)
        } else {
            Ok(text.to_owned())
        }
    }

    fn email(text: &str) -> Result<String, InvalidAnswer> {
        if text.contains('@') {
            Ok(text.to_owned())
        } else {
            Err(InvalidAnswer::NotEmail)
        }
    }

    fn form(script: Script) -> Result<Form<Script>> {
        Form::intro(script, "holst login")
    }

    #[rstest]
    #[case::three("739", "▪▪▪")]
    fn secret_never_reaches_the_screen(
        #[case] code: &'static str,
        #[case] mask: &str,
    ) -> Result<()> {
        let script = Script::new(
            TerminalKind::Live,
            chars(code).chain([Keystroke::Enter]),
            [],
        );
        let form = form(script)?;
        let typed = form.ask("Code", QuestionKind::LoginCode, &not_empty)?;
        let raw = form.terminal().written();
        assert_eq!(typed, code);
        assert!(!raw.contains(code), "the code went out: {raw:?}");
        let printed = without_escapes(&raw);
        assert!(!printed.chars().any(|c| c.is_ascii_digit()), "{printed:?}");
        assert!(screen(&raw).ends_with(&format!("◇  Code\n│  {mask}\n│")));
        assert_eq!(form.terminal().hidden_reads(), 1);
        Ok(())
    }

    #[rstest]
    #[case::backspace_and_ctrl_u(
        chars("12345").chain([Keystroke::Backspace, Keystroke::ClearBefore]).chain(chars("98")).collect(),
        "98",
        "▪▪"
    )]
    #[case::pasted_with_spaces(chars("123 456").collect(), "123456", "▪▪▪▪▪▪")]
    fn typed_code_and_its_mask(
        #[case] strokes: Vec<Keystroke>,
        #[case] answer: &str,
        #[case] mask: &str,
    ) -> Result<()> {
        let strokes = strokes.into_iter().chain([Keystroke::Enter]);
        let form = form(Script::new(TerminalKind::Live, strokes, []))?;
        assert_eq!(
            form.ask("Code", QuestionKind::LoginCode, &not_empty)?,
            answer
        );
        let shown = screen(&form.terminal().written());
        assert!(
            shown.ends_with(&format!("◇  Code\n│  {mask}\n│")),
            "{shown}"
        );
        Ok(())
    }

    #[test]
    fn wrong_answers_are_asked_again_and_warnings_do_not_pile_up() -> Result<()> {
        let strokes = chars("bad")
            .chain([Keystroke::Enter])
            .chain([Keystroke::Enter])
            .chain(chars("you@example.com"))
            .chain([Keystroke::Enter]);
        let form = form(Script::new(TerminalKind::Live, strokes, []))?;
        assert_eq!(
            form.ask("Email", QuestionKind::Text, &email)?,
            "you@example.com"
        );
        let shown = screen(&form.terminal().written());
        assert_eq!(shown.matches('▲').count(), 0, "{shown}");
        assert!(
            shown.ends_with("◇  Email\n│  you@example.com\n│"),
            "{shown}"
        );
        Ok(())
    }

    #[test]
    fn retry_after_a_refused_code_starts_empty() -> Result<()> {
        let form = form(Script::new(
            TerminalKind::Live,
            chars("2").chain([Keystroke::Enter]),
            [],
        ))?;
        let again = form.ask_again(
            Question {
                label: "Code",
                kind: QuestionKind::LoginCode,
            },
            &not_empty,
            InvalidAnswer::WrongCode,
        )?;
        assert_eq!(again, "2");
        let raw = form.terminal().written();
        assert!(raw.contains("▲ that code did not match"), "{raw:?}");
        Ok(())
    }

    #[rstest]
    #[case::ctrl_c(Script::new(TerminalKind::Live, [Keystroke::Interrupt], []), QuestionKind::Text, "interrupted")]
    #[case::esc(Script::new(TerminalKind::Live, [Keystroke::Cancel], []), QuestionKind::Text, "login cancelled")]
    #[case::ctrl_d(Script::new(TerminalKind::Live, [Keystroke::EndOfInput], []), QuestionKind::Text, "login cancelled")]
    #[case::stdin_ended(Script::new(TerminalKind::Redirected, [], []), QuestionKind::Text, "no input: stdin ended")]
    #[case::piped_wrong_answer(Script::new(TerminalKind::Redirected, [], ["nope"]), QuestionKind::Text, "Email: enter an address")]
    #[case::echo_not_hidden(
        Script::new(TerminalKind::Dumb, [], ["a@b.c"]).without_hidden_echo(),
        QuestionKind::LoginCode,
        "could not hide the input"
    )]
    fn question_fails(
        #[case] script: Script,
        #[case] field: QuestionKind,
        #[case] message: &str,
    ) -> Result<()> {
        let err = form(script)?.ask("Email", field, &email).err();
        let text = err.as_ref().map(ToString::to_string).unwrap_or_default();
        assert!(text.starts_with(message), "{err:?}");
        Ok(())
    }

    #[test]
    fn blind_secret_is_read_with_the_echo_off() -> Result<()> {
        let form = form(Script::new(TerminalKind::Dumb, [], ["", "123456"]))?;
        assert_eq!(
            form.ask("Code", QuestionKind::LoginCode, &not_empty)?,
            "123456"
        );
        assert_eq!(form.terminal().hidden_reads(), 2);
        let raw = form.terminal().written();
        assert!(raw.contains("▲  this cannot be empty"), "{raw:?}");
        assert!(!raw.contains("123456"), "{raw:?}");
        Ok(())
    }

    #[tokio::test]
    async fn waiting_line_is_drawn_and_removed_only_when_live() -> Result<()> {
        let live = form(Script::new(TerminalKind::Live, [], []))?;
        let piped = form(Script::new(TerminalKind::Redirected, [], []))?;
        let work = || async { Ok::<_, Error>(7) };
        assert_eq!(live.wait("Checking", work()).await?, 7);
        assert_eq!(piped.wait("Checking", work()).await?, 7);
        assert!(live.terminal().written().contains("◒  Checking…"));
        assert!(!piped.terminal().written().contains('◒'));
        assert!(!screen(&live.terminal().written()).contains('◒'));
        Ok(())
    }

    #[tokio::test]
    async fn ctrl_c_during_a_wait_closes_the_form() -> Result<()> {
        let form = form(Script::new(TerminalKind::Live, [], []).pressing_ctrl_c())?;
        let err = form
            .wait("Signing in", std::future::pending::<Result<()>>())
            .await
            .err();
        assert!(matches!(err, Some(Error::Interrupted)), "{err:?}");
        form.cancel(&Error::Interrupted)?;
        let shown = screen(&form.terminal().written());
        assert!(!shown.contains('◒'), "{shown}");
        assert!(shown.ends_with("└  login cancelled"), "{shown}");
        assert_eq!(
            crate::exit::code(&Error::Interrupted.shown()),
            crate::exit::EXIT_INTERRUPTED
        );
        Ok(())
    }

    #[tokio::test]
    async fn ctrl_c_before_a_step_draws_nothing_of_it() -> Result<()> {
        let form = form(
            Script::new(TerminalKind::Live, chars("a@b.c"), []).interrupted_before(),
        )?;
        let waited = form
            .wait("Reading", std::future::ready(Ok::<_, Error>(1)))
            .await;
        assert!(matches!(waited, Err(Error::Interrupted)), "{waited:?}");
        let asked = form.ask("Email", QuestionKind::Text, &email);
        assert!(matches!(asked, Err(Error::Interrupted)), "{asked:?}");
        let shown = screen(&form.terminal().written());
        assert!(!shown.contains("Reading"), "{shown}");
        assert!(!shown.contains("Email"), "{shown}");
        Ok(())
    }

    #[rstest]
    #[case::live(TerminalKind::Live, "\r\x1b[2K┌")]
    #[case::piped(TerminalKind::Redirected, "┌")]
    fn intro_quiets_and_starts_on_a_clean_line(
        #[case] kind: TerminalKind,
        #[case] opening: &str,
    ) -> Result<()> {
        let form = form(Script::new(kind, [], []))?;
        let written = form.terminal().written();
        assert!(form.terminal().was_quieted());
        assert!(written.starts_with(opening), "{written:?}");
        assert_eq!(screen(&written).lines().next(), Some("┌   holst login"));
        Ok(())
    }

    #[tokio::test]
    async fn finished_work_wins_over_ctrl_c() -> Result<()> {
        let form = form(Script::new(TerminalKind::Live, [], []).pressing_ctrl_c())?;
        let work = async { Ok::<_, Error>(7) };
        assert_eq!(form.wait("Checking", work).await?, 7);
        Ok(())
    }

    #[rstest]
    #[case::live(Script::new(TerminalKind::Live, chars("12").chain([Keystroke::Enter]), []), 1)]
    #[case::blind_secret(Script::new(TerminalKind::Dumb, [], ["12"]), 1)]
    #[case::piped(Script::new(TerminalKind::Redirected, [], ["12"]), 0)]
    fn unread_input_is_dropped_after_an_answer(
        #[case] script: Script,
        #[case] discards: usize,
    ) -> Result<()> {
        let form = form(script)?;
        assert_eq!(form.ask("Code", QuestionKind::LoginCode, &not_empty)?, "12");
        assert_eq!(form.terminal().discards(), discards);
        Ok(())
    }

    #[test]
    fn long_text_answer_is_redrawn_with_an_ellipsis() -> Result<()> {
        let long = "a".repeat(200);
        let strokes: Vec<Keystroke> = long
            .chars()
            .map(Keystroke::Char)
            .chain([Keystroke::Enter])
            .collect();
        let form = form(Script::new(TerminalKind::Live, strokes, []))?;
        assert_eq!(form.ask("Email", QuestionKind::Text, &not_empty)?, long);
        assert!(form.terminal().written().contains('…'));
        Ok(())
    }

    #[test]
    fn a_bare_form_quiets_the_terminal_and_writes_nothing() {
        let form = Form::bare(Script::new(TerminalKind::Live, [], []));
        assert!(form.terminal().was_quieted());
        assert_eq!(form.terminal().written(), "");
    }

    #[test]
    fn a_flipped_answer_and_the_final_erase_repaint_the_three_rows() -> Result<()> {
        let strokes = [Keystroke::Tab, Keystroke::Char('n')];
        let form = Form::bare(Script::new(TerminalKind::Live, strokes, []));
        assert!(!form.confirm("Go?", true)?);
        let written = form.terminal().written();
        let repaints = written.matches("\r\x1b[3A\x1b[J").count();
        assert_eq!(repaints, 2, "{written:?}");
        assert!(written.ends_with("◇  Go?\n│  No\n│\n"), "{written:?}");
        assert_eq!(form.terminal().discards(), 1);
        Ok(())
    }
}
