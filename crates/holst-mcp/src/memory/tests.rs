use crate::memory::{
    McpMemory,
    alias::{Aliases, UnknownAlias, alias_text, expand_json},
};

const ID: &str = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b";

fn uuid(number: usize) -> String {
    format!("00000001-0000-4000-8000-{number:012x}")
}

#[test]
fn aliases_are_stable_and_numbered_from_one() {
    let state = McpMemory::default();
    assert_eq!([ID, ID, "x"].map(|id| state.alias(id)), ["@1", "@1", "@2"]);
}

#[test]
fn aliases_stay_within_their_capacity() {
    let capacity = 128;
    let aliases = Aliases::with_capacity(capacity);
    let ids: Vec<String> = (0..capacity * 5).map(uuid).collect();
    let numbers: Vec<usize> = ids.iter().map(|id| aliases.intern(id)).collect();
    assert_eq!(aliases.held(), (capacity, capacity));
    let (old, new) = numbers.split_at(numbers.len() - capacity);
    assert!(
        old.iter().all(|&n| aliases.id(n).is_none()),
        "an evicted @N expands"
    );
    let newest = ids.get(ids.len() - capacity..).unwrap_or_default();
    assert!(
        new.iter()
            .zip(newest)
            .all(|(&n, id)| aliases.id(n).as_deref() == Some(id.as_str())),
        "a kept @N lost its id"
    );
    let again = ids.first().map(|id| aliases.intern(id));
    assert_eq!(
        again,
        Some(capacity * 5 + 1),
        "an evicted id did not get a new number"
    );
    assert_eq!(aliases.held(), (capacity, capacity));
}

#[rstest::fixture]
fn state() -> McpMemory {
    let state = McpMemory::default();
    state.alias(ID);
    state.alias("x");
    state
}

#[rstest::rstest]
#[case::alias("@1", ID)]
#[case::alias_in_spaces(" @1 ", ID)]
#[case::eight_digits_are_an_id_start("@12345678", "@12345678")]
#[case::hex_is_an_id_start("@dbc79d73", "@dbc79d73")]
#[case::text_keeps_spaces(" Plans ", " Plans ")]
#[case::handle_keeps_spaces("@team ", "@team ")]
fn expand(state: McpMemory, #[case] reference: &str, #[case] expected: &str) {
    assert_eq!(state.expand(reference).ok().as_deref(), Some(expected));
}

#[rstest::rstest]
#[case::unknown("@99", "@99")]
#[case::zero("@0", "@0")]
#[case::longest_alias("@9999999", "@9999999")]
#[case::unknown_in_spaces(" @7 ", "@7")]
fn expand_unknown(state: McpMemory, #[case] reference: &str, #[case] alias: &str) {
    assert_eq!(
        state.expand(reference).err().map(|err| err.to_string()),
        Some(format!(
            "{alias} is not from this session (did the server restart?): take an \
                 @N from a fresh search or list"
        ))
    );
}

#[rstest::rstest]
#[case::segment_not_query("/gapi/channel/@1/users?x=@1", &format!("/gapi/channel/{ID}/users?x=@1"))]
#[case::inside_segment("/a/b@1c/x", "/a/b@1c/x")]
#[case::spaces_kept("/a/ b /@1", &format!("/a/ b /{ID}"))]
#[case::empty("", "")]
#[case::root("/", "/")]
#[case::alias_only("@1", ID)]
#[case::empty_segments("//@2//", "//x//")]
#[case::trailing_slash("@1/@2/", &format!("{ID}/x/"))]
#[case::empty_query("/a?", "/a?")]
#[case::query_untouched("?q=@1", "?q=@1")]
#[case::second_question_mark("/@2?a=1?b=2", "/x?a=1?b=2")]
fn expand_path(state: McpMemory, #[case] path: &str, #[case] expanded: &str) {
    assert_eq!(
        crate::memory::alias::expand_path(path, |s| state.expand(s)).as_deref(),
        Ok(expanded)
    );
}

#[rstest::rstest]
fn first_unknown_alias_in_path_is_an_error(state: McpMemory) {
    let unknown = crate::memory::alias::expand_path("/a/@1/@7/b", |s| state.expand(s));
    assert_eq!(unknown, Err(UnknownAlias { alias: "@7".into() }));
}

#[rstest::rstest]
#[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
fn only_whole_alias_strings_are_expanded_in_json(
    state: McpMemory,
) -> Result<(), UnknownAlias> {
    // imperative: `expand_json` rewrites the value in place.
    let mut body = serde_json::json!({
        "ids": ["@1"], "mail": "ivan@example.org", "file": "logo@2x.png", "handle": "@team",
        "spaced": "@team ", "id": "@2", "text": "@ 1",
    });
    expand_json(&mut body, &|s| state.expand(s))?;
    let expected = serde_json::json!({
        "ids": [ID], "mail": "ivan@example.org", "file": "logo@2x.png", "handle": "@team",
        "spaced": "@team ", "id": "x", "text": "@ 1",
    });
    assert_eq!(body, expected);
    Ok(())
}

#[test]
fn inline_keeps_everything_but_whole_uuids() {
    let text = format!("«{ID}»,{ID} f{ID} {ID}0 😀");
    assert_eq!(
        alias_text(&text, |id| format!("<{}>", id.len())),
        format!("«<36>»,<36> f{ID} {ID}0 😀")
    );
}

#[test]
#[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
fn invalidate_keeps_aliases() -> Result<(), UnknownAlias> {
    let state = McpMemory::default();
    state.alias(ID);
    state.invalidate_all();
    assert_eq!(state.expand("@1")?, ID);
    Ok(())
}
