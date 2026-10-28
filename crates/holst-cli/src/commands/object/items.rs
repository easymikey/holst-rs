//! `--items FILE`: the file or stdin, read into the array the command parses.

use serde_json::{Map, Value};

use crate::{
    commands::read_source,
    error::{Error, Result},
};

/// Replaces the file named by `items` with the objects in it.
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
        serde_json::from_str(text).map_err(Error::ItemsNotJson)
    } else {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<Value>, _>>()
            .map(Value::Array)
            .map_err(Error::ItemsNotJson)
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
        assert!(matches!(parse("{\"text\": "), Err(Error::ItemsNotJson(_))));
        assert!(matches!(parse("[{}"), Err(Error::ItemsNotJson(_))));
    }
}
