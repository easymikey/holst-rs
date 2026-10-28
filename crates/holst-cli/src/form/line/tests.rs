use rstest::rstest;
use zeroize::Zeroizing;

use crate::form::line::{
    CAPACITY,
    Csi,
    EditorStep,
    EscapeTail,
    Keystroke,
    QuestionKind,
    TypedAnswer,
    with_room,
};

fn typed(text: &str) -> impl Iterator<Item = Keystroke> + '_ {
    text.chars().map(|c| match c {
        '\n' => Keystroke::Enter,
        c => Keystroke::Char(c),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum End {
    Submitted(String),
    Typing(String),
    Cancelled,
}

fn run(field: QuestionKind, strokes: impl IntoIterator<Item = Keystroke>) -> End {
    let last = strokes
        .into_iter()
        .try_fold(TypedAnswer::new(field), |line, stroke| {
            match line.press(stroke) {
                EditorStep::Typing(line) => Ok(line),
                EditorStep::Submit(text) => Err(End::Submitted(text.to_string())),
                EditorStep::Quit(_) => Err(End::Cancelled),
            }
        });
    last.map_or_else(|end| end, |line| End::Typing(line.text().to_owned()))
}

fn typing(
    field: QuestionKind,
    strokes: impl IntoIterator<Item = Keystroke>,
) -> TypedAnswer {
    strokes
        .into_iter()
        .fold(TypedAnswer::new(field), |line, stroke| {
            match line.press(stroke) {
                EditorStep::Typing(line) => line,
                EditorStep::Submit(_) | EditorStep::Quit(_) => TypedAnswer::new(field),
            }
        })
}

#[test]
fn enter_submits_trimmed_text() {
    assert_eq!(
        run(QuestionKind::Text, typed(" you@example.com \n")),
        End::Submitted("you@example.com".into())
    );
}

#[rstest]
#[case::ascii("abc", "ab")]
#[case::cyrillic("код", "ко")]
#[case::combining("cafe\u{301}", "caf")]
#[case::family_emoji("a👨‍👩‍👧", "a")]
#[case::flag("x🇺🇦", "x")]
fn backspace_removes_one_grapheme(#[case] text: &str, #[case] left: &str) {
    let strokes = typed(text).chain([Keystroke::Backspace]);
    assert_eq!(run(QuestionKind::Text, strokes), End::Typing(left.into()));
}

#[test]
fn backspace_on_empty_line_does_nothing() {
    assert_eq!(
        run(
            QuestionKind::LoginCode,
            [Keystroke::Backspace, Keystroke::Backspace]
        ),
        End::Typing(String::new())
    );
}

#[test]
fn ctrl_u_clears_and_ctrl_w_removes_a_word() {
    assert_eq!(
        run(
            QuestionKind::Text,
            typed("abc").chain([Keystroke::ClearBefore])
        ),
        End::Typing(String::new())
    );
    assert_eq!(
        run(
            QuestionKind::Text,
            typed("one two  ").chain([Keystroke::DeleteWord])
        ),
        End::Typing("one ".into())
    );
    assert_eq!(
        run(
            QuestionKind::Text,
            typed("single").chain([Keystroke::DeleteWord])
        ),
        End::Typing(String::new())
    );
}

#[test]
fn code_drops_whitespace_of_a_paste() {
    assert_eq!(
        run(QuestionKind::LoginCode, typed(" 123 456\t\n")),
        End::Submitted("123456".into())
    );
}

#[test]
fn email_keeps_inner_spaces_and_trims_edges() {
    assert_eq!(
        run(QuestionKind::Text, typed("  a b@c.d  \n")),
        End::Submitted("a b@c.d".into())
    );
}

#[test]
fn pasted_newline_submits_the_first_line() {
    assert_eq!(
        run(QuestionKind::LoginCode, typed("123456\n999")),
        End::Submitted("123456".into())
    );
}

#[rstest]
#[case::ctrl_c(Keystroke::Interrupt)]
#[case::esc(Keystroke::Cancel)]
#[case::ctrl_d_on_empty(Keystroke::EndOfInput)]
fn cancel_strokes(#[case] stroke: Keystroke) {
    assert_eq!(run(QuestionKind::LoginCode, [stroke]), End::Cancelled);
}

#[test]
fn ctrl_d_on_a_typed_line_is_ignored() {
    assert_eq!(
        run(
            QuestionKind::LoginCode,
            typed("12").chain([Keystroke::EndOfInput])
        ),
        End::Typing("12".into())
    );
}

#[test]
fn keys_without_an_effect_leave_no_trace() {
    let strokes = typed("1")
        .chain([
            Keystroke::Ignored,
            Keystroke::Char('\u{7}'),
            Keystroke::Escape(EscapeTail::Done),
        ])
        .chain(typed("2"));
    assert_eq!(run(QuestionKind::Text, strokes), End::Typing("12".into()));
}

#[test]
fn rest_of_escape_sequences_is_swallowed() {
    let strokes = typed("a")
        .chain([Keystroke::Escape(EscapeTail::Csi(Csi::default()))])
        .chain(typed("5C"))
        .chain([Keystroke::Escape(EscapeTail::Ss3)])
        .chain(typed("Pb"));
    assert_eq!(run(QuestionKind::Text, strokes), End::Typing("ab".into()));
}

#[test]
fn enter_during_an_escape_still_submits() {
    let strokes = typed("ab").chain([
        Keystroke::Escape(EscapeTail::Csi(Csi::default())),
        Keystroke::Enter,
    ]);
    assert_eq!(
        run(QuestionKind::Text, strokes),
        End::Submitted("ab".into())
    );
}

#[test]
fn mask_length_counts_graphemes() {
    let line = typing(QuestionKind::LoginCode, typed("12👨‍👩‍👧"));
    assert_eq!(line.graphemes(), 3);
}

#[test]
fn answer_longer_than_the_reserve_survives() {
    let long = "x".repeat(CAPACITY * 3 + 1);
    let strokes = typed(&long).chain([Keystroke::Enter]);
    assert_eq!(
        run(QuestionKind::Password, strokes),
        End::Submitted(long.clone())
    );
    assert!(with_room(Zeroizing::new(String::new()), 4).capacity() >= 4);
}

#[test]
fn placeholders_say_whether_the_answer_is_hidden() {
    assert!(QuestionKind::LoginCode.placeholder().contains("email"));
    assert!(QuestionKind::Password.placeholder().contains("hidden"));
    assert!(!QuestionKind::Text.placeholder().contains("hidden"));
}

fn after_keys(field: QuestionKind, text: &str, keys: &[Keystroke]) -> End {
    run(field, typed(text).chain(keys.iter().copied()))
}

#[rstest]
#[case::left_then_type("ac", &[Keystroke::Left, Keystroke::Char('b')], "abc")]
#[case::home_then_type("bc", &[Keystroke::Home, Keystroke::Char('a')], "abc")]
#[case::end_after_home("ab", &[Keystroke::Home, Keystroke::End, Keystroke::Char('c')], "abc")]
#[case::right_at_end("ab", &[Keystroke::Right, Keystroke::Char('c')], "abc")]
#[case::left_at_start("b", &[Keystroke::Home, Keystroke::Left, Keystroke::Char('a')], "ab")]
#[case::backspace_in_middle("abxc", &[Keystroke::Left, Keystroke::Backspace], "abc")]
#[case::backspace_at_start("abc", &[Keystroke::Home, Keystroke::Backspace], "abc")]
#[case::delete_in_middle("abxc", &[Keystroke::Left, Keystroke::Left, Keystroke::Delete], "abc")]
#[case::delete_at_end("abc", &[Keystroke::Delete], "abc")]
#[case::ctrl_u_before_cursor("abc", &[Keystroke::Left, Keystroke::ClearBefore], "c")]
#[case::ctrl_k_after_cursor("abc", &[Keystroke::Left, Keystroke::Left, Keystroke::ClearAfter], "a")]
#[case::ctrl_w_before_cursor("one two three", &[Keystroke::WordLeft, Keystroke::DeleteWord], "one three")]
#[case::cyrillic_left("кд", &[Keystroke::Left, Keystroke::Char('о')], "код")]
#[case::family_left("a👨‍👩‍👧", &[Keystroke::Left, Keystroke::Char('b')], "ab👨‍👩‍👧")]
#[case::flag_left("🇺🇦🇫🇷", &[Keystroke::Left, Keystroke::Backspace], "🇫🇷")]
#[case::combining_delete("cafe\u{301}!", &[Keystroke::Left, Keystroke::Left, Keystroke::Delete], "caf!")]
#[case::up_down_ignored("ab", &[Keystroke::Ignored, Keystroke::Left, Keystroke::Ignored, Keystroke::Char('x')], "axb")]
fn editing_at_the_cursor(
    #[case] text: &str,
    #[case] keys: &[Keystroke],
    #[case] result: &str,
) {
    assert_eq!(
        after_keys(QuestionKind::Text, text, keys),
        End::Typing(result.into())
    );
}

#[rstest]
#[case::word_left("mock@example.com", &[Keystroke::WordLeft, Keystroke::Char('x')], "mock@example.xcom")]
#[case::two_words_left("mock@example.com", &[Keystroke::WordLeft, Keystroke::WordLeft, Keystroke::Char('x')], "mock@xexample.com")]
#[case::word_right("mock@example.com", &[Keystroke::Home, Keystroke::WordRight, Keystroke::Char('x')], "mockx@example.com")]
#[case::word_right_past_separator("ab  cd", &[Keystroke::Home, Keystroke::WordRight, Keystroke::WordRight, Keystroke::Char('x')], "ab  cdx")]
#[case::word_left_at_start("ab", &[Keystroke::Home, Keystroke::WordLeft, Keystroke::Char('x')], "xab")]
fn moving_by_words(
    #[case] text: &str,
    #[case] keys: &[Keystroke],
    #[case] result: &str,
) {
    assert_eq!(
        after_keys(QuestionKind::Text, text, keys),
        End::Typing(result.into())
    );
}

#[rstest]
#[case::ctrl_left("1;5", 'D', "|a b")]
#[case::alt_left("1;3", 'D', "|a b")]
#[case::shift_left("1;2", 'D', "a| b")]
#[case::alt_right("1;3", 'C', "a b|")]
#[case::home_tilde("1", '~', "|a b")]
#[case::end_tilde("4", '~', "a b|")]
#[case::delete_tilde("3", '~', "a |")]
#[case::unknown_tilde("9", '~', "a |b")]
fn csi_tails_finish_into_keys(
    #[case] digits: &str,
    #[case] last: char,
    #[case] result: &str,
) {
    let csi = digits.chars().fold(Csi::default(), Csi::feed);
    let keys = [
        Keystroke::Left,
        Keystroke::Escape(EscapeTail::Csi(csi)),
        Keystroke::Char(last),
        Keystroke::Char('|'),
    ];
    assert_eq!(
        after_keys(QuestionKind::Text, "a b", &keys),
        End::Typing(result.into())
    );
}

#[test]
fn ss3_home_and_end() {
    let keys = [
        Keystroke::Escape(EscapeTail::Ss3),
        Keystroke::Char('H'),
        Keystroke::Char('x'),
    ];
    assert_eq!(
        after_keys(QuestionKind::Text, "ab", &keys),
        End::Typing("xab".into())
    );
}

#[test]
fn code_edits_at_the_cursor() {
    let keys = [Keystroke::Left, Keystroke::Backspace, Keystroke::Enter];
    assert_eq!(
        after_keys(QuestionKind::LoginCode, "123456", &keys),
        End::Submitted("12346".into())
    );
}

#[test]
fn merging_graphemes_keep_the_cursor_on_a_boundary() {
    let strokes = typed("👨👩").chain([Keystroke::Left, Keystroke::Char('\u{200d}')]);
    let line = typing(QuestionKind::Text, strokes);
    assert_eq!((line.graphemes(), line.cursor()), (1, line.text().len()));
}

mod laws {
    use proptest::prelude::{
        Just,
        ProptestConfig,
        Strategy,
        prop,
        prop_assert,
        prop_assert_eq,
        prop_oneof,
        proptest,
    };
    use unicode_segmentation::UnicodeSegmentation as _;

    use crate::form::line::{EditorStep, Keystroke, QuestionKind, TypedAnswer};

    fn stroke() -> impl Strategy<Value = Keystroke> {
        prop_oneof![
            prop::sample::select(vec![
                'a', 'я', ' ', '@', '\u{301}', '\u{200d}', '👨', '🇺', '界'
            ])
            .prop_map(Keystroke::Char),
            Just(Keystroke::Backspace),
            Just(Keystroke::Delete),
            Just(Keystroke::Left),
            Just(Keystroke::Right),
            Just(Keystroke::WordLeft),
            Just(Keystroke::WordRight),
            Just(Keystroke::Home),
            Just(Keystroke::End),
            Just(Keystroke::ClearBefore),
            Just(Keystroke::ClearAfter),
            Just(Keystroke::DeleteWord),
        ]
    }

    fn typing(line: TypedAnswer, stroke: Keystroke) -> TypedAnswer {
        match line.press(stroke) {
            EditorStep::Typing(line) => line,
            EditorStep::Submit(_) | EditorStep::Quit(_) => {
                TypedAnswer::new(QuestionKind::Text)
            }
        }
    }

    fn on_boundary(line: &TypedAnswer) -> bool {
        line.text()
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .chain([line.text().len()])
            .any(|at| at == line.cursor())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, rng_seed: proptest::test_runner::RngSeed::Fixed(0x5eed), ..ProptestConfig::default() })]

        #[test]
        fn cursor_stays_on_a_boundary(strokes in prop::collection::vec(stroke(), 0..40)) {
            let line = strokes.into_iter().fold(TypedAnswer::new(QuestionKind::Text), typing);
            prop_assert!(line.cursor() <= line.text().len());
            prop_assert!(on_boundary(&line), "{:?} at {}", line.text(), line.cursor());
        }

        #[test]
        fn left_then_right_comes_back(text in "[a-zя @.]{1,20}", back in 0usize..20) {
            let line = text.chars().map(Keystroke::Char).fold(TypedAnswer::new(QuestionKind::Text), typing);
            let moved = std::iter::repeat_n(Keystroke::Left, back).fold(line, typing);
            let at = moved.cursor();
            let back_again = typing(typing(moved, Keystroke::Left), Keystroke::Right);
            prop_assert!(at == 0 || back_again.cursor() == at);
        }

        #[test]
        fn home_then_typing_is_a_prefix(text in "[a-z]{0,20}", prefix in "[a-z]{1,10}") {
            let line = text.chars().map(Keystroke::Char).fold(TypedAnswer::new(QuestionKind::Text), typing);
            let line = prefix.chars().map(Keystroke::Char).fold(typing(line, Keystroke::Home), typing);
            prop_assert_eq!(line.text(), format!("{prefix}{text}"));
        }
    }
}
