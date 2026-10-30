#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use yrs::{
    Doc,
    Map as _,
    ReadTxn as _,
    Transact as _,
    Update,
    updates::decoder::Decode as _,
};

use super::{add, built};
use crate::{
    domain::{board::tests::sticker_under, change::value::NewObject, geometry::Point},
    error::Result,
};

#[rstest]
#[case::v2("objects", "objects_v3")]
#[case::v3("objects_v3", "objects")]
fn a_write_lands_under_the_objects_root_of_the_document(
    #[case] root: &str,
    #[case] other: &str,
) -> Result<()> {
    let document = sticker_under(root, "seed-1")?;
    let new = NewObject::Sticker {
        text: "n".into(),
        at: Point::new(500.0, 500.0)?,
        size: None,
        color: None,
    };
    let update = built(&document, &add(new))?.update;
    let replica = Doc::new();
    replica
        .transact_mut()
        .apply_update(Update::decode_v2(&update)?)?;
    let transaction = replica.transact();
    assert_eq!(
        transaction.get_map(root).map(|map| map.len(&transaction)),
        Some(1)
    );
    assert!(transaction.get_map(other).is_none());
    Ok(())
}
