//! `--items FILE`: the file or stdin, read into the array the command parses.

use holst_core::error::input::JsonSpot;
use serde_json::{Map, Value};

use crate::{
    commands::read_source,
    error::{Error, Result},
};

/// Replaces the file named by `items` with the objects in it.
#[expect(
    clippy::disallowed_methods,
    reason = "`-` names the process's stdin as the items file"
)]
pub(super) fn read(arguments: &mut Map<String, Value>) -> Result<()> {
    let source = arguments
        .get("items")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(source) = source {
        let items = parse(&read_source(&source, &mut std::io::stdin())?)?;
        arguments.insert("items".to_owned(), items);
    }
    Ok(())
}

/// Returns the values of a JSON array, or of JSON lines.
fn parse(text: &str) -> Result<Value> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim_start().starts_with('[') {
        serde_json::from_str(text).map_err(|error| Error::ItemsNotJson {
            spot: JsonSpot::of(&error, text),
            error,
        })
    } else {
        text.lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(at, line)| {
                serde_json::from_str(line).map_err(|error| Error::ItemsNotJson {
                    spot: JsonSpot {
                        line: at + 1,
                        ..JsonSpot::of(&error, line)
                    },
                    error,
                })
            })
            .collect::<Result<Vec<Value>>>()
            .map(Value::Array)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{commands::object::items::parse, error::Error};

    #[test]
    fn an_array_and_json_lines_give_the_same_items() {
        let expected = json!([{ "text": "a" }, { "text": "b", "at": "1,2" }]);
        assert_eq!(
            parse(r#"[{"text":"a"},{"text":"b","at":"1,2"}]"#).ok(),
            Some(expected.clone())
        );
        assert_eq!(
            parse("{\"text\":\"a\"}\n\n{\"text\":\"b\",\"at\":\"1,2\"}\n").ok(),
            Some(expected)
        );
    }

    #[test]
    fn a_byte_order_mark_and_an_empty_file_are_fine() {
        assert_eq!(parse("\u{feff}[{}]").ok(), Some(json!([{}])));
        assert_eq!(parse("").ok(), Some(json!([])));
    }

    #[test]
    fn text_that_is_not_json_is_refused() {
        assert!(matches!(
            parse("{\"text\": "),
            Err(Error::ItemsNotJson { .. })
        ));
        assert!(matches!(parse("[{}"), Err(Error::ItemsNotJson { .. })));
    }

    fn spot(text: &str) -> Option<(usize, usize, String)> {
        match parse(text) {
            Err(Error::ItemsNotJson { spot, .. }) => {
                Some((spot.line, spot.column, spot.text))
            }
            Ok(_) | Err(_) => None,
        }
    }

    #[test]
    fn the_error_of_an_array_knows_its_line_and_column() {
        let text = "[\n  {\"a\": 1},\n  {\"a\": }\n]";
        assert_eq!(spot(text), Some((3, 9, "  {\"a\": }".to_owned())));
    }

    #[test]
    fn the_error_of_json_lines_knows_the_line_in_the_file() {
        let text = "{\"a\":1}\n\n{\"a\":2}\n{\"a\": }\n";
        assert_eq!(spot(text), Some((4, 7, "{\"a\": }".to_owned())));
    }
}
