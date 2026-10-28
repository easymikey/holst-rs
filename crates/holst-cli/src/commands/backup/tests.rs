use holst_core::{
    command::{backup::Fetched, board::picture::Delivery},
    domain::backup::Backups,
};
use rstest::rstest;
use serde_json::json;

use crate::commands::backup::render::{fetched_text, listing_text};

fn backups(listed: &serde_json::Value) -> Backups {
    serde_json::from_value(json!({ "result": listed }))
        .unwrap_or(Backups::Listed { result: Vec::new() })
}

#[test]
fn backups_are_listed_as_key_provider_and_date() {
    let listed = backups(&json!([
        { "key": "k2", "server_provider_type": "hud", "date": "2026-10-02" },
        { "key": "k1", "date": "2026-10-01" },
    ]));
    assert_eq!(
        listing_text(&listed),
        "k2 · hud · 2026-10-02\nk1 · 2026-10-01"
    );
}

#[test]
fn no_backups_list_as_none() {
    assert_eq!(listing_text(&backups(&json!([]))), "(empty)");
}

#[rstest]
#[case::gzip(300, 900, "saved 300 bytes (900 before gzip) to /tmp/b.ydoc.gz")]
#[case::plain(900, 900, "saved 900 bytes to /tmp/b.ydoc.gz")]
fn a_saved_backup_says_its_size(
    #[case] bytes: usize,
    #[case] before: usize,
    #[case] expected: &str,
) {
    let fetched = Fetched {
        key: "k".to_owned(),
        uncompressed_bytes: before,
        delivery: Delivery::Written {
            file: "/tmp/b.ydoc.gz".into(),
            bytes,
        },
    };
    assert_eq!(fetched_text(&fetched), expected);
}
