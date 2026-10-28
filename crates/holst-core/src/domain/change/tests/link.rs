use rstest::rstest;
use serde_json::Value;
use yrs::Doc;

use super::{ME, board as seeded, edit, fields_of};
use crate::{
    domain::{
        change::{
            build::build,
            value::{ChangeSources, LinkTo, ObjectPatch},
        },
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::Result,
};

const SHARE: &str = "https://holst.example/share/b/b1";

fn link(document: &Doc, link_to: LinkTo) -> Result<()> {
    let ids = SequentialIds::new("r");
    build(
        document,
        &edit(
            "seed-2".into(),
            ObjectPatch {
                link_to: Some(link_to),
                ..ObjectPatch::default()
            },
        ),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(9),
            ids: &ids,
        },
    )
    .map(drop)
}

fn stored(document: &Doc) -> Option<String> {
    fields_of(document, "seed-2")
        .get("linkTo")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[rstest]
#[case::address(LinkTo::Url("http://a.io/x".into()), "http://a.io/x")]
#[case::clear(LinkTo::Clear, "")]
#[case::object(
    LinkTo::Object { reference: "seed-1".into(), share: SHARE.into() },
    "https://holst.example/share/b/b1?objectId=seed-1"
)]
fn the_link_is_stored_as_the_web_client_stores_it(
    #[case] asked: LinkTo,
    #[case] wanted: &str,
) -> Result<()> {
    let document = seeded()?;
    link(&document, asked)?;
    assert_eq!(stored(&document).as_deref(), Some(wanted));
    Ok(())
}

#[test]
fn a_link_to_an_object_that_is_not_there_is_refused_and_writes_nothing() -> Result<()> {
    let document = seeded()?;
    let refused = link(
        &document,
        LinkTo::Object {
            reference: "nowhere".into(),
            share: SHARE.into(),
        },
    );
    assert!(refused.is_err());
    assert_eq!(stored(&document), None);
    Ok(())
}
