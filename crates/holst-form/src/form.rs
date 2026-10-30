//! The login form à la `@clack/prompts`; a secret shows only as `▪`.

use std::{convert::Infallible, ops::ControlFlow, time::Duration};

use zeroize::Zeroizing;

use crate::{
    coloring::Coloring,
    draw::{self, NoteTone, QuestionFooter},
    error::{FormError, Reason, Result},
    line::{Cancellation, Decoder, EditorStep, Keystroke, QuestionKind, TypedAnswer},
    prompt::{Choice, Confirm, MultiSelect, Prompt, Select, Step},
    signals,
    terminal::{Terminal, TerminalKind},
};

const SPIN_STEP: Duration = Duration::from_millis(80);
/// How many options of a select or multi-select show at once.
const CHOICE_ROWS: usize = 8;

enum KeyReadEnd {
    Typed(Zeroizing<String>),
    GaveUp {
        masked: String,
        cancellation: Cancellation,
    },
}

/// Shows the cursor again when dropped, also while a panic unwinds.
struct CursorHidden<'a, T: Terminal>(&'a Form<T>);

impl<T: Terminal> Drop for CursorHidden<'_, T> {
    fn drop(&mut self) {
        self.0.write(draw::SHOW_CURSOR).unwrap_or_default();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Question {
    pub label: &'static str,
    pub kind: QuestionKind,
}

#[derive(Debug)]
pub struct Form<T: Terminal> {
    terminal: T,
    coloring: Coloring,
    _quiet: T::QuietTerminal,
}

impl<T: Terminal> Form<T> {
    /// A form without an intro line: one question inside a command.
    pub fn bare(term: T) -> Self {
        Self {
            _quiet: term.quiet(),
            coloring: Coloring::colored_if(term.has_color()),
            terminal: term,
        }
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn intro(term: T, title: &str) -> Result<Self> {
        let form = Self::bare(term);
        form.write_live(&draw::clear_line())?;
        form.write(&draw::intro(form.coloring, title))
            .map(|()| form)
    }

    fn not_interrupted(&self) -> Result<()> {
        if self.terminal.interrupts() == 0 {
            Ok(())
        } else {
            Err(FormError::Interrupted)
        }
    }

    #[must_use]
    pub const fn terminal(&self) -> &T {
        &self.terminal
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn ask<V>(
        &self,
        label: &'static str,
        kind: QuestionKind,
        parse: &impl Fn(&str) -> std::result::Result<V, Reason>,
    ) -> Result<V> {
        self.ask_with(Question { label, kind }, parse, QuestionFooter::Plain)
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn ask_again<V>(
        &self,
        question: Question,
        parse: &impl Fn(&str) -> std::result::Result<V, Reason>,
        invalid: Reason,
    ) -> Result<V> {
        match self.terminal.kind() {
            TerminalKind::Live | TerminalKind::Dumb => {
                self.ask_with(question, parse, QuestionFooter::Warn(invalid))
            }
            TerminalKind::Redirected => Err(FormError::InvalidAnswer {
                question: question.label,
                reason: invalid,
            }),
        }
    }

    /// Asks, runs `attempt` on the answer, and asks again while `retryable`
    /// gives a reason, `tries` at most; a redirected input is asked once.
    /// # Errors
    /// The terminal failed, or the person stopped the form.
    /// The last attempt error, if tries run out, is in the inner `Result`.
    pub async fn ask_until<V, A, E>(
        &self,
        question: Question,
        parse: &impl Fn(&str) -> std::result::Result<V, Reason>,
        attempt: impl AsyncFn(&V) -> std::result::Result<A, E>,
        retryable: impl Fn(&E) -> Option<Reason>,
        tries: u8,
    ) -> Result<std::result::Result<A, E>> {
        let answer = self.ask_with(question, parse, QuestionFooter::Plain)?;
        self.try_answer(question, answer, parse, &attempt, &retryable, tries)
            .await
    }

    async fn try_answer<V, A, E>(
        &self,
        question: Question,
        answer: V,
        parse: &impl Fn(&str) -> std::result::Result<V, Reason>,
        attempt: &impl AsyncFn(&V) -> std::result::Result<A, E>,
        retryable: &impl Fn(&E) -> Option<Reason>,
        tries: u8,
    ) -> Result<std::result::Result<A, E>> {
        let outcome = self.wait("Signing in", attempt(&answer)).await?;
        let again =
            outcome.as_ref().err().and_then(retryable).filter(|_| {
                tries > 1 && self.terminal.kind() != TerminalKind::Redirected
            });
        match again {
            None => Ok(outcome),
            Some(reason) => {
                let next = self.ask_again(question, parse, reason)?;
                Box::pin(self.try_answer(
                    question,
                    next,
                    parse,
                    attempt,
                    retryable,
                    tries - 1,
                ))
                .await
            }
        }
    }

    fn ask_with<V>(
        &self,
        question: Question,
        parse: &impl Fn(&str) -> std::result::Result<V, Reason>,
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
    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn confirm(&self, label: &str, default: bool) -> Result<bool> {
        let yes = self.run(label, Confirm::new(default))?;
        self.step(label, &Confirm::new(yes).summary()).map(|()| yes)
    }

    /// Asks for one of `options`; the answer is its place.
    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn select(&self, label: &str, options: Vec<Choice>) -> Result<usize> {
        let labels: Vec<String> =
            options.iter().map(|option| option.label.clone()).collect();
        let prompt = Select::new(options, CHOICE_ROWS)
            .ok_or(FormError::NoInput { what: "a choice" })?;
        let at = self.run(label, prompt)?;
        self.step(label, labels.get(at).map_or("", String::as_str))
            .map(|()| at)
    }

    /// Asks for any of `options`; the answer is their places in option order.
    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn multiselect(
        &self,
        label: &str,
        options: Vec<Choice>,
        picked: Vec<bool>,
        required: bool,
    ) -> Result<Vec<usize>> {
        let prompt = MultiSelect::new(options, picked, required, CHOICE_ROWS)
            .ok_or(FormError::NoInput { what: "a choice" })?;
        let shown = prompt.clone();
        let places = self.run(label, prompt)?;
        self.step(label, &shown.labelled(&places)).map(|()| places)
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
            TerminalKind::Redirected => Err(FormError::NoInput { what: "an answer" }),
        }
    }

    fn run_keys<P: Prompt>(&self, label: &str, prompt: P) -> Result<P::Answer> {
        let done = self
            .terminal
            .hidden(|_| {
                let _shown_again = CursorHidden(self);
                self.write(draw::HIDE_CURSOR)
                    .and_then(|()| self.press_keys(label, prompt))
            })
            .map_err(FormError::Terminal)?;
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
                Err(error) => ControlFlow::Break(Err(FormError::Terminal(error))),
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
            ControlFlow::Continue(_) => Err(FormError::Cancelled),
        }
    }

    /// A typed line stands for keys, as the prompt reads it.
    fn run_lines<P: Prompt>(&self, label: &str, prompt: P) -> Result<P::Answer> {
        let rows: String = prompt
            .listing()
            .iter()
            .flat_map(|row| [row.as_str(), "\n"])
            .collect();
        let line = self
            .terminal
            .write(&format!("◆  {label}\n{rows}"))
            .and_then(|()| self.terminal.line())
            .inspect(|_| self.terminal.discard_typeahead())
            .map_err(FormError::ReadInput)?
            .ok_or(FormError::Cancelled)?;
        let flow = prompt
            .typed(&line)
            .into_iter()
            .try_fold(prompt, |prompt, key| match prompt.press(key) {
                Step::Editing(prompt) => ControlFlow::Continue(prompt),
                Step::Submit(answer) => ControlFlow::Break(Ok(answer)),
                Step::Quit(quit) => ControlFlow::Break(Err(stopped_by(quit))),
            });
        match flow {
            ControlFlow::Break(done) => done,
            ControlFlow::Continue(_) => Err(FormError::Cancelled),
        }
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn step(&self, label: &str, value: &str) -> Result<()> {
        self.write(&draw::answered(self.coloring, label, value))
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn write_box(&self, title: &str, lines: &[(NoteTone, String)]) -> Result<()> {
        self.write(&draw::boxed(
            self.coloring,
            title,
            lines,
            self.terminal.width(),
        ))
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub async fn wait<W>(
        &self,
        label: &str,
        work: impl Future<Output = W>,
    ) -> Result<W> {
        // Listen before the line shows, so an early Ctrl-C reaches this wait.
        let interrupt = self.terminal.interrupt();
        self.not_interrupted()?;
        self.write_live(&draw::waiting(self.coloring, label))?;
        let out = signals::or_interrupted(
            async {
                tokio::select! {
                    out = work => Ok(out),
                    never = self.spin(label) => match never {},
                }
            },
            interrupt,
            || Err(FormError::Interrupted),
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
        // imperative: the frames repeat until the request ends and `select!`
        // drops this
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

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn outro(&self, message: &str) -> Result<()> {
        self.write(&draw::outro(self.coloring, message))
    }

    /// # Errors
    /// The terminal failed, or the person stopped the form.
    pub fn cancel(&self, message: &str, causes: &[String]) -> Result<()> {
        self.write(&draw::error_outro(self.coloring, message, causes))
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
                match keystroke.map_err(FormError::Terminal).map(|keystroke| {
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
            ControlFlow::Continue(_) => Err(FormError::Cancelled),
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
                .map_err(FormError::EchoNotHidden)?,
            QuestionKind::Text => self
                .terminal
                .write(&question)
                .and_then(|()| self.terminal.line())
                .inspect(|_| self.terminal.discard_typeahead()),
        };
        line.map_err(FormError::ReadInput)?
            .map(|text| clean_answer(kind, &text))
            .ok_or(FormError::Cancelled)
    }

    fn read_piped(
        &self,
        label: &'static str,
        kind: QuestionKind,
    ) -> Result<Zeroizing<String>> {
        self.terminal
            .line()
            .map_err(FormError::ReadInput)?
            .map(|text| clean_answer(kind, &Zeroizing::new(text)))
            .ok_or(FormError::NoInput { what: label })
    }

    fn write_live(&self, text: &str) -> Result<()> {
        match self.terminal.kind() {
            TerminalKind::Live => self.write(text),
            TerminalKind::Dumb | TerminalKind::Redirected => Ok(()),
        }
    }

    fn write(&self, text: &str) -> Result<()> {
        self.terminal.write(text).map_err(FormError::Terminal)
    }
}

fn stopped_by(quit: Cancellation) -> FormError {
    match quit {
        Cancellation::Interrupt => FormError::Interrupted,
        Cancellation::Cancel => FormError::Cancelled,
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

    use std::panic::{AssertUnwindSafe, catch_unwind};

    use rstest::rstest;

    use crate::{
        coloring::Coloring,
        error::{FormError, Reason, Result},
        fake::{Script, screen, without_escapes},
        form::{Form, Question},
        line::{Keystroke, QuestionKind},
        prompt::{Choice, Prompt, Step},
        terminal::TerminalKind,
    };

    const EMPTY: Reason = Reason("this cannot be empty");
    const NOT_EMAIL: Reason = Reason("enter an address like you@example.com");
    const WRONG_CODE: Reason = Reason("that code did not match");

    fn chars(text: &'static str) -> impl Iterator<Item = Keystroke> {
        text.chars().map(Keystroke::Char)
    }

    fn not_empty(text: &str) -> Result<String, Reason> {
        if text.is_empty() {
            Err(EMPTY)
        } else {
            Ok(text.to_owned())
        }
    }

    fn email(text: &str) -> Result<String, Reason> {
        if text.contains('@') {
            Ok(text.to_owned())
        } else {
            Err(NOT_EMAIL)
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
            WRONG_CODE,
        )?;
        assert_eq!(again, "2");
        let raw = form.terminal().written();
        assert!(raw.contains("▲ that code did not match"), "{raw:?}");
        Ok(())
    }

    #[rstest]
    #[case::ctrl_c(Script::new(TerminalKind::Live, [Keystroke::Interrupt], []), QuestionKind::Text, "interrupted")]
    #[case::esc(Script::new(TerminalKind::Live, [Keystroke::Cancel], []), QuestionKind::Text, "cancelled")]
    #[case::ctrl_d(Script::new(TerminalKind::Live, [Keystroke::EndOfInput], []), QuestionKind::Text, "cancelled")]
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
        let work = || async { 7 };
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
            .wait("Signing in", std::future::pending::<()>())
            .await
            .err();
        assert!(matches!(err, Some(FormError::Interrupted)), "{err:?}");
        form.cancel("login cancelled", &[])?;
        let shown = screen(&form.terminal().written());
        assert!(!shown.contains('◒'), "{shown}");
        assert!(shown.ends_with("└  login cancelled"), "{shown}");
        Ok(())
    }

    #[tokio::test]
    async fn ctrl_c_before_a_step_draws_nothing_of_it() -> Result<()> {
        let form = form(
            Script::new(TerminalKind::Live, chars("a@b.c"), []).interrupted_before(),
        )?;
        let waited = form.wait("Reading", std::future::ready(1)).await;
        assert!(matches!(waited, Err(FormError::Interrupted)), "{waited:?}");
        let asked = form.ask("Email", QuestionKind::Text, &email);
        assert!(matches!(asked, Err(FormError::Interrupted)), "{asked:?}");
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
        let work = async { 7 };
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

    fn plans() -> Vec<Choice> {
        ["Plans A", "Plans B", "Plans C"]
            .into_iter()
            .map(|label| Choice {
                label: label.to_owned(),
                hint: None,
            })
            .collect()
    }

    fn chosen(result: &Result<usize>) -> &'static str {
        match result {
            Ok(0) => "first",
            Ok(1) => "second",
            Ok(_) => "later",
            Err(FormError::Interrupted) => "interrupted",
            Err(FormError::Cancelled) => "cancelled",
            Err(FormError::NoInput { .. }) => "no input",
            Err(_) => "other",
        }
    }

    #[rstest]
    #[case::down_enter(TerminalKind::Live, vec![Keystroke::Down, Keystroke::Enter], &[], "second")]
    #[case::enter(TerminalKind::Live, vec![Keystroke::Enter], &[], "first")]
    #[case::wraps_up(TerminalKind::Live, vec![Keystroke::Up, Keystroke::Up, Keystroke::Enter], &[], "second")]
    #[case::ctrl_c(TerminalKind::Live, vec![Keystroke::Interrupt], &[], "interrupted")]
    #[case::escape(TerminalKind::Live, vec![Keystroke::Cancel], &[], "cancelled")]
    #[case::line_number(TerminalKind::Dumb, vec![], &["2"], "second")]
    #[case::line_last(TerminalKind::Dumb, vec![], &["3"], "later")]
    #[case::line_out_of_range(TerminalKind::Dumb, vec![], &["4"], "cancelled")]
    #[case::line_words(TerminalKind::Dumb, vec![], &["B"], "cancelled")]
    #[case::line_end_of_input(TerminalKind::Dumb, vec![], &[], "cancelled")]
    #[case::piped(TerminalKind::Redirected, vec![], &["2"], "no input")]
    fn a_select_answers_with_the_place_of_the_option(
        #[case] kind: TerminalKind,
        #[case] strokes: Vec<Keystroke>,
        #[case] lines: &'static [&'static str],
        #[case] expected: &str,
    ) {
        let form = Form::bare(Script::new(kind, strokes, lines.iter().copied()));
        let result = form.select("Which?", plans());
        assert_eq!(chosen(&result), expected, "{result:?}");
    }

    #[test]
    fn no_options_make_no_question() {
        let form = Form::bare(Script::new(TerminalKind::Live, [Keystroke::Enter], []));
        assert_eq!(chosen(&form.select("Which?", Vec::new())), "no input");
        assert_eq!(form.terminal().written(), "");
    }

    #[test]
    fn a_line_select_lists_the_options_and_keeps_the_answer() -> Result<()> {
        let form = Form::bare(Script::new(TerminalKind::Dumb, [], ["2"]));
        assert_eq!(form.select("Which?", plans())?, 1);
        assert_eq!(
            form.terminal().written(),
            "◆  Which?\n  1) Plans A\n  2) Plans B\n  3) Plans C\n◇  Which?\n│  Plans B\n│\n"
        );
        Ok(())
    }

    fn places(result: &Result<Vec<usize>>) -> String {
        match result {
            Ok(places) => format!("{places:?}"),
            Err(FormError::Interrupted) => "interrupted".to_owned(),
            Err(FormError::Cancelled) => "cancelled".to_owned(),
            Err(FormError::NoInput { .. }) => "no input".to_owned(),
            Err(_) => "other".to_owned(),
        }
    }

    const SPACE: Keystroke = Keystroke::Char(' ');

    #[rstest]
    #[case::pick_beside_the_marked(TerminalKind::Live, vec![SPACE, Keystroke::Down, Keystroke::Down, SPACE, Keystroke::Enter], &[], "[0, 1, 2]")]
    #[case::unpick_the_marked(TerminalKind::Live, vec![Keystroke::Down, SPACE, Keystroke::Enter], &[], "[]")]
    #[case::start_picked(TerminalKind::Live, vec![Keystroke::Enter], &[], "[1]")]
    #[case::all(TerminalKind::Live, vec![Keystroke::Char('a'), Keystroke::Enter], &[], "[0, 1, 2]")]
    #[case::ctrl_c(TerminalKind::Live, vec![Keystroke::Interrupt], &[], "interrupted")]
    #[case::escape(TerminalKind::Live, vec![Keystroke::Cancel], &[], "cancelled")]
    #[case::line_numbers(TerminalKind::Dumb, vec![], &["1 3"], "[0, 2]")]
    #[case::line_keeps_the_marked(TerminalKind::Dumb, vec![], &[""], "[1]")]
    #[case::line_words(TerminalKind::Dumb, vec![], &["all"], "cancelled")]
    #[case::line_end_of_input(TerminalKind::Dumb, vec![], &[], "cancelled")]
    #[case::piped(TerminalKind::Redirected, vec![], &["1"], "no input")]
    fn a_multiselect_answers_with_the_places_of_the_options(
        #[case] kind: TerminalKind,
        #[case] strokes: Vec<Keystroke>,
        #[case] lines: &'static [&'static str],
        #[case] expected: &str,
    ) {
        let form = Form::bare(Script::new(kind, strokes, lines.iter().copied()));
        let result = form.multiselect("Which?", plans(), vec![false, true], false);
        assert_eq!(places(&result), expected, "{result:?}");
    }

    #[test]
    fn a_required_multiselect_warns_and_waits_for_a_pick() -> Result<()> {
        let strokes = [Keystroke::Enter, SPACE, Keystroke::Enter];
        let form = Form::bare(Script::new(TerminalKind::Live, strokes, []));
        assert_eq!(form.multiselect("Which?", plans(), vec![], true)?, [0]);
        let written = form.terminal().written();
        assert_eq!(
            written
                .matches("Please select at least one option.")
                .count(),
            1
        );
        assert!(
            written.ends_with("◇  Which?\n│  Plans A\n│\n"),
            "{written:?}"
        );
        Ok(())
    }

    #[test]
    fn a_multiselect_repaints_the_active_rows_and_shows_the_summary() -> Result<()> {
        let form = Form::bare(Script::new(
            TerminalKind::Live,
            [SPACE, Keystroke::Enter],
            [],
        ));
        assert_eq!(form.multiselect("Which?", plans(), vec![], false)?, [0]);
        let written = form.terminal().written();
        let repaints = written.matches("\r\x1b[5A\x1b[J").count();
        assert_eq!(repaints, 2, "{written:?}");
        assert!(written.contains("│ ›◼ Plans A"), "{written:?}");
        Ok(())
    }

    #[test]
    fn nothing_picked_shows_none() -> Result<()> {
        let form = Form::bare(Script::new(TerminalKind::Live, [Keystroke::Enter], []));
        assert_eq!(
            form.multiselect("Which?", plans(), vec![], false)?,
            Vec::<usize>::new()
        );
        assert!(
            form.terminal()
                .written()
                .ends_with("◇  Which?\n│  none\n│\n")
        );
        Ok(())
    }

    #[test]
    fn no_options_make_no_multiselect_question() {
        let form = Form::bare(Script::new(TerminalKind::Live, [Keystroke::Enter], []));
        let result = form.multiselect("Which?", Vec::new(), Vec::new(), false);
        assert_eq!(places(&result), "no input");
        assert_eq!(form.terminal().written(), "");
    }

    #[test]
    fn a_line_multiselect_lists_the_options_with_marks() -> Result<()> {
        let form = Form::bare(Script::new(TerminalKind::Dumb, [], ["2 3"]));
        assert_eq!(
            form.multiselect("Which?", plans(), vec![true], false)?,
            [1, 2]
        );
        assert_eq!(
            form.terminal().written(),
            "◆  Which?\n  1) [x] Plans A\n  2) [ ] Plans B\n  3) [ ] Plans C\n  numbers to pick, e.g. 1 3; Enter keeps the marked\n◇  Which?\n│  Plans B, Plans C\n│\n"
        );
        Ok(())
    }

    #[rstest]
    #[case::picked(vec![Keystroke::Down, Keystroke::Enter])]
    #[case::ctrl_c(vec![Keystroke::Interrupt])]
    #[case::escape(vec![Keystroke::Cancel])]
    fn the_cursor_is_hidden_while_a_select_is_open_and_back_after(
        #[case] strokes: Vec<Keystroke>,
    ) {
        let form = Form::bare(Script::new(TerminalKind::Live, strokes, []));
        let result = form.select("Which?", plans());
        let written = form.terminal().written();
        let at = |mark: &str| written.find(mark);
        let first_row = at("Plans A");
        assert_eq!(
            (
                at("\x1b[?25l") < first_row,
                first_row < at("\x1b[?25h"),
                written.matches("\x1b[?25h").count()
            ),
            (true, true, 1),
            "{result:?} {written:?}"
        );
    }

    struct Broken;

    impl Prompt for Broken {
        type Answer = ();

        fn press(self, _: Keystroke) -> Step<Self, ()> {
            Step::Submit(())
        }

        #[expect(clippy::panic, reason = "the prompt breaks on purpose")]
        fn view(&self, _: Coloring, _: Option<usize>) -> Vec<String> {
            panic!("the prompt broke")
        }

        fn summary(&self) -> String {
            String::new()
        }

        fn listing(&self) -> Vec<String> {
            Vec::new()
        }

        fn typed(&self, _: &str) -> Vec<Keystroke> {
            Vec::new()
        }
    }

    #[test]
    fn the_cursor_is_back_when_a_key_prompt_panics() {
        let form = Form::bare(Script::new(TerminalKind::Live, [], []));
        let broke = catch_unwind(AssertUnwindSafe(|| form.run_keys("Which?", Broken)));
        let written = form.terminal().written();
        assert_eq!(
            (broke.is_err(), written.matches("\x1b[?25h").count()),
            (true, 1),
            "{written:?}"
        );
    }

    const CODE: Question = Question {
        label: "Code",
        kind: QuestionKind::LoginCode,
    };

    fn right_code(code: &str) -> Result<(), Reason> {
        match code {
            "123456" => Ok(()),
            _ => Err(WRONG_CODE),
        }
    }

    #[rstest]
    #[case::second_try_takes(Script::new(TerminalKind::Live, chars("000000").chain([Keystroke::Enter]).chain(chars("123456")).chain([Keystroke::Enter]), []), 3, Ok(()), 1)]
    #[case::tries_run_out(Script::new(TerminalKind::Live, chars("000000").chain([Keystroke::Enter]).chain(chars("111111")).chain([Keystroke::Enter]), []), 2, Err(WRONG_CODE), 1)]
    #[case::piped_is_asked_once(Script::new(TerminalKind::Redirected, [], ["000000", "123456"]), 3, Err(WRONG_CODE), 0)]
    #[tokio::test]
    async fn ask_until_asks_again_only_while_it_can(
        #[case] script: Script,
        #[case] tries: u8,
        #[case] expected: Result<(), Reason>,
        #[case] warnings: usize,
    ) -> Result<()> {
        let form = form(script)?;
        let done = form
            .ask_until(
                CODE,
                &not_empty,
                async |code: &String| right_code(code),
                |_| Some(WRONG_CODE),
                tries,
            )
            .await?;
        let raw = form.terminal().written();
        assert_eq!(done, expected);
        assert_eq!(
            raw.matches("▲ that code did not match").count(),
            warnings,
            "{raw:?}"
        );
        Ok(())
    }
}
