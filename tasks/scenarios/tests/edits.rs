//! Changing the objects of a board.

use holst_scenarios::{
    FixtureObject,
    Given,
    Holst,
    ItemsFile,
    ObjectCall,
    Observation,
    ScenarioError,
    Surface,
    board_of,
    given,
    then,
    withhold_deleting,
};
use rstest::rstest;
use serde_json::{Value, json};

fn two_notes() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![
            FixtureObject::Sticker("Buy milk".into()),
            FixtureObject::FreeText("Roadmap".into()),
        ],
    )
}

fn frame_and_note() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![
            FixtureObject::Frame("Q4".into()),
            FixtureObject::Sticker("Buy milk".into()),
        ],
    )
}

#[rstest]
#[tokio::test]
async fn a_changed_text_shows_on_the_next_read(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    holst
        .on(surface)
        .change_object_text("Plans", &sticker, "Buy oat milk")
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects(&[("sticker", "Buy oat milk"), ("text", "Roadmap")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_at_before_an_id_start_names_an_object(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    // Fixture ids share their first 8 characters; the core test has the short form.
    let listed = format!("@{sticker}");
    holst
        .on(surface)
        .change_object_text("Plans", &listed, "Buy oat milk")
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects(&[("sticker", "Buy oat milk"), ("text", "Roadmap")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_moved_object_stays_where_it_was_put(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    holst
        .on(surface)
        .move_object("Plans", &sticker, (400, 50))
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_at("Buy milk", (400, 50));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_removed_object_is_gone_on_the_next_read(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    holst.on(surface).remove_object("Plans", &sticker).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[("text", "Roadmap")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_sticker_added_to_a_frame_belongs_to_it(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = frame_and_note().start().await?;
    holst
        .on(surface)
        .add_sticker("Plans", "Call Ann", Some("Q4"))
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_in_a_frame("sticker", "Call Ann");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_sticker_moved_into_a_frame_joins_it(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = frame_and_note().start().await?;
    let sticker = holst.fixture_object("Plans", 2)?;
    let before = holst.on(surface).objects("Plans").await?;
    then(&before).lists_no_object_in_a_frame();
    holst
        .on(surface)
        .move_object("Plans", &sticker, (200, 300))
        .await?;
    let after = holst.on(surface).objects("Plans").await?;
    then(&after).lists_object_in_a_frame("sticker", "Buy milk");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_grouped_sticker_moves_and_stays_in_its_group(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Frame("Q4".into()),
                FixtureObject::Shape("Group".into()).set("type", json!("group")),
                FixtureObject::Sticker("Buy milk".into()).inside(2),
            ],
        )
        .start()
        .await?;
    let sticker = holst.fixture_object("Plans", 3)?;
    holst
        .on(surface)
        .move_object("Plans", &sticker, (5000, 5000))
        .await?;
    let after = holst.on(surface).objects("Plans").await?;
    then(&after).lists_object_at("Buy milk", (5000, 5000));
    then(&after).lists_object_in_a_frame("sticker", "Buy milk");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn another_users_sticker_stays_on_a_restricted_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    withhold_deleting(&holst, "Plans", "co_owner", "editor");
    let theirs = holst.fixture_object("Plans", 1)?;
    let observation = holst.on(surface).remove_object("Plans", &theirs).await?;
    then(&observation)
        .failed_saying("made by someone else")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects(&[("sticker", "Buy milk"), ("text", "Roadmap")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_frame_is_not_removed_with_its_objects_unconfirmed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = frame_and_note().start().await?;
    let frame = holst.fixture_object("Plans", 1)?;
    holst
        .on(surface)
        .add_sticker("Plans", "Call Ann", Some("Q4"))
        .await?;
    let observation = holst.on(surface).remove_object("Plans", &frame).await?;
    then(&observation)
        .failed_saying("removing the frame and 1 object with it")
        .signalled_as(surface.permission_denied());
    let read = holst.on(surface).objects("Plans").await?;
    // What is left matters here, not the listing order.
    then(&read).lists_objects_in_any_order(&[
        ("frame", "Q4"),
        ("sticker", "Call Ann"),
        ("sticker", "Buy milk"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_removal_takes_a_frame_with_its_objects(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = frame_and_note().start().await?;
    let frame = holst.fixture_object("Plans", 1)?;
    holst
        .on(surface)
        .add_sticker("Plans", "Call Ann", Some("Q4"))
        .await?;
    holst
        .on(surface)
        .confirming()
        .remove_object("Plans", &frame)
        .await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects(&[("sticker", "Buy milk")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_preview_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    holst
        .on(surface)
        .dry_running()
        .add_sticker("Plans", "Call Ann", None)
        .await?;
    board_of(&holst).got_no_update();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[("sticker", "Buy milk"), ("text", "Roadmap")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_read_only_board_refuses_a_change(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .read_only_board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Buy milk".into())],
        )
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .add_sticker("Plans", "Call Ann", None)
        .await?;
    then(&observation)
        .failed_saying("read-only")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects(&[("sticker", "Buy milk")]);
    Ok(())
}

fn texts(observation: &Observation) -> Vec<String> {
    if let Observation::Objects(rows) = observation {
        rows.iter().map(|row| row.text.clone()).collect()
    } else {
        Vec::new()
    }
}

/// What the refusal of a shape says: the command line has clap list the shapes
/// below its first line, the tool lists them in the message.
const fn shape_words(surface: Surface) -> &'static str {
    match surface {
        Surface::Cli => "blob",
        Surface::Mcp => "ellipse",
    }
}

/// Returns `value`, or the id of the second object of the fixture for `@2`.
fn resolved(holst: &Holst, value: &str) -> Result<String, ScenarioError> {
    if value == "@2" {
        holst.fixture_object("Plans", 2)
    } else {
        Ok(value.to_owned())
    }
}

fn five_notes() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        ["one", "two", "three", "four", "five"]
            .map(|text| FixtureObject::Sticker(text.into()))
            .to_vec(),
    )
}

// Frame Q4 holds `In`; `Out` is far away.
fn frame_with_one_inside() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![
            FixtureObject::Frame("Q4".into()).at(0, 0).sized(400, 400),
            FixtureObject::Sticker("In".into()).at(50, 50).inside(1),
            FixtureObject::Sticker("Out".into()).at(3000, 3000),
        ],
    )
}

#[rstest]
#[case::sticker("sticker", "Call Ann", &[])]
#[case::text("text", "Call Ann", &[])]
#[case::shape("shape", "Call Ann", &[("shape", "ellipse")])]
#[case::frame("frame", "Call Ann", &[])]
#[tokio::test]
async fn each_object_kind_can_be_added(
    #[case] kind: &'static str,
    #[case] text: &str,
    #[case] flags: &[(&'static str, &'static str)],
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = flags
        .iter()
        .fold(ObjectCall::add(kind), |call, (name, value)| {
            call.with(name, *value)
        });
    holst.on(surface).object("Plans", call.text(text)).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[
        ("sticker", "Buy milk"),
        ("text", "Roadmap"),
        (kind, text),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_arrow_joins_two_objects(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("arrow")
        .with("from", holst.fixture_object("Plans", 1)?)
        .with("to", holst.fixture_object("Plans", 2)?);
    holst.on(surface).object("Plans", call).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_arrow_between("Buy milk", "Roadmap");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_arrow_to_an_unknown_object_is_not_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("arrow")
        .with("from", holst.fixture_object("Plans", 1)?)
        .with("to", "deadbeefdeadbeef");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("not found on the board")
        .signalled_as(surface.not_found());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_misspelt_shape_is_refused_with_the_shapes_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("shape")
        .text("Launch")
        .with("shape", "blob");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying(shape_words(surface))
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_new_object_takes_a_free_corner(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Buy milk".into()).at(0, 0)],
        )
        .start()
        .await?;
    holst
        .on(surface)
        .add_sticker("Plans", "Call Ann", None)
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_apart("Call Ann", "Buy milk");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_size_and_colour_are_kept(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("sticker")
        .text("Call Ann")
        .with("size", "200x100")
        .with("color", "yellow4");
    holst.on(surface).object("Plans", call).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_sized("Call Ann", "200x100");
    then(&observation).lists_object_coloured("Call Ann", "yellow4");
    Ok(())
}

fn three_stickers() -> Vec<Value> {
    ["A", "B", "C"].map(|text| json!({ "text": text })).to_vec()
}

#[rstest]
#[tokio::test]
async fn many_objects_go_in_one_board_write(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("sticker").items(three_stickers());
    holst.on(surface).object("Plans", call).await?;
    board_of(&holst).got_one_update();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[
        ("sticker", "A"),
        ("sticker", "B"),
        ("sticker", "C"),
        ("sticker", "Buy milk"),
        ("text", "Roadmap"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn items_can_mix_kinds_and_name_earlier_items(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("sticker").items(vec![
        json!({ "text": "First" }),
        json!({ "kind": "shape", "text": "Second" }),
        json!({ "kind": "arrow", "from": "#1", "to": "#2" }),
    ]);
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[
        ("sticker", "Buy milk"),
        ("text", "Roadmap"),
        ("sticker", "First"),
        ("shape", "Second"),
        ("arrow", ""),
    ]);
    then(&observation).lists_arrow_between("First", "Second");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_bad_item_adds_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("sticker").items(vec![
        json!({ "text": "A" }),
        json!({ "text": "B", "color": "not-a-colour" }),
        json!({ "text": "C" }),
    ]);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_this_many_objects(2);
    Ok(())
}

// Only the CLI reads files.
#[rstest]
#[case::array(ItemsFile::Array)]
#[case::lines(ItemsFile::Lines)]
#[case::stdin(ItemsFile::Stdin)]
#[tokio::test]
async fn items_come_as_json_lines_or_an_array(
    #[case] via: ItemsFile,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::add("sticker")
        .items(three_stickers())
        .items_via(via);
    holst.on(Surface::Cli).object("Plans", call).await?;
    let observation = holst.on(Surface::Cli).objects("Plans").await?;
    then(&observation).lists_this_many_objects(5);
    Ok(())
}

// The cap of 100 is the tool's.
#[tokio::test]
async fn too_many_items_are_refused() -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let items = (0..101).map(|number| json!({ "text": number })).collect();
    let call = ObjectCall::add("sticker").items(items);
    let observation = holst.on(Surface::Mcp).object("Plans", call).await?;
    then(&observation)
        .failed_saying("100")
        .signalled_as(Surface::Mcp.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_text_filter_finds_objects_ignoring_case(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::list().with("contains", "MILK");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).lists_objects(&[("sticker", "Buy milk")]);
    Ok(())
}

#[rstest]
#[case::rectangle("rectangle", "0,0,400,400")]
#[case::frame("frame", "Q4")]
#[case::object("object", "@2")]
#[tokio::test]
async fn objects_are_selected_by_rectangle_frame_or_object(
    #[case] flag: &'static str,
    #[case] value: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = frame_with_one_inside().start().await?;
    let value = resolved(&holst, value)?;
    let call = ObjectCall::list().with(flag, value).with("padding", "0");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).lists_text("In", true);
    then(&observation).lists_text("Out", false);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_long_object_list_is_paged(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = five_notes().start().await?;
    let page = |offset: &'static str, limit: &'static str| {
        ObjectCall::list()
            .with("offset", offset)
            .with("limit", limit)
    };
    let first = holst.on(surface).object("Plans", page("0", "2")).await?;
    let second = holst.on(surface).object("Plans", page("2", "2")).await?;
    let all = holst.on(surface).object("Plans", page("0", "0")).await?;
    let (first, second, all) = (texts(&first), texts(&second), texts(&all));
    assert_eq!((first.len(), second.len(), all.len()), (2, 2, 5));
    assert!(
        first.iter().all(|text| !second.contains(text)),
        "{first:?} {second:?}"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_cached_object_list_is_refreshed_on_request(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    holst.on(surface).objects("Plans").await?;
    holst.edit_board_behind_its_back(
        "Plans",
        vec![FixtureObject::Sticker("Late".into())],
    )?;
    let fresh = holst
        .on(surface)
        .object("Plans", ObjectCall::list().switched("refresh"))
        .await?;
    then(&fresh).lists_text("Late", true);
    Ok(())
}

/// `offline` is the command line's only: MCP always asks the server.
#[tokio::test]
async fn an_offline_object_list_is_the_cached_copy() -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    holst.on(Surface::Cli).objects("Plans").await?;
    holst.edit_board_behind_its_back(
        "Plans",
        vec![FixtureObject::Sticker("Late".into())],
    )?;
    let copy = holst
        .on(Surface::Cli)
        .object(
            &holst.board_id("Plans")?,
            ObjectCall::list().switched("offline"),
        )
        .await?;
    then(&copy).lists_text("Late", false);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_edited_object_moves_in_the_stacking_order(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Sticker("A".into()).stacked(1),
                FixtureObject::Sticker("B".into()).stacked(2),
            ],
        )
        .start()
        .await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("order", "front");
    holst.on(surface).object("Plans", call).await?;
    // the listing is in reading order; the stacking is what the board stores
    let (a, b) = (
        holst.stacking_of("Plans", 1)?,
        holst.stacking_of("Plans", 2)?,
    );
    assert!(a > b, "A should be above B: {a:?} {b:?}");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn object_tags_are_set_and_cleared(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    let tag = |tag: &'static str| {
        ObjectCall::edit()
            .with("object", sticker.clone())
            .with("tag", tag)
    };
    holst.on(surface).object("Plans", tag("urgent")).await?;
    let tagged = holst.on(surface).objects("Plans").await?;
    then(&tagged).lists_object_tagged("Buy milk", &["urgent"]);
    holst.on(surface).object("Plans", tag("")).await?;
    let cleared = holst.on(surface).objects("Plans").await?;
    then(&cleared).lists_object_tagged("Buy milk", &[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn many_objects_are_edited_in_one_write(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::edit().items(vec![
        json!({ "object": holst.fixture_object("Plans", 1)?, "text": "Buy oat milk" }),
        json!({ "object": holst.fixture_object("Plans", 2)?, "text": "Plan" }),
    ]);
    holst.on(surface).object("Plans", call).await?;
    board_of(&holst).got_one_update();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Buy oat milk"), ("text", "Plan")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_edit_without_an_object_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::edit().with("text", "Buy oat milk");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[case::edit(ObjectCall::edit().with("text", "x"))]
#[case::remove(ObjectCall::remove())]
#[case::copy(ObjectCall::copy())]
#[tokio::test]
async fn an_unknown_object_is_not_found(
    #[case] call: ObjectCall,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let observation = holst
        .on(surface)
        .object("Plans", call.with("object", "deadbeefdeadbeef"))
        .await?;
    then(&observation)
        .failed_saying("not found on the board")
        .signalled_as(surface.not_found());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_object_link_selects_the_object(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let link = holst.object_link("Plans", 1)?;
    let call = ObjectCall::edit()
        .with("object", link)
        .with("text", "Buy oat milk");
    holst.on(surface).object("Plans", call).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[
        ("sticker", "Buy oat milk"),
        ("text", "Roadmap"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_copy_keeps_the_content_with_new_ids(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::copy().with("object", holst.fixture_object("Plans", 1)?);
    holst.on(surface).object("Plans", call).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_two_objects_with_text("Buy milk");
    then(&observation).lists_this_many_objects(3);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_copy_goes_to_the_asked_corner(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::copy()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("at", "700,80");
    holst.on(surface).object("Plans", call).await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_at("Buy milk", (700, 80));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_frame_name_that_fits_two_frames_lists_them(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Frame("Q4 plan".into()),
                FixtureObject::Frame("Q4 review".into()),
            ],
        )
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .add_sticker("Plans", "Call Ann", Some("Q4"))
        .await?;
    then(&observation)
        .failed_saying("Q4 plan")
        .failed_saying("Q4 review")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[case::list(ObjectCall::list())]
#[case::add(ObjectCall::add("sticker").text("x"))]
#[case::edit(ObjectCall::edit().with("object", "deadbeef").with("text", "x"))]
#[case::remove(ObjectCall::remove().with("object", "deadbeef"))]
#[case::copy(ObjectCall::copy().with("object", "deadbeef"))]
#[case::get(ObjectCall::get().with("object", "deadbeef"))]
#[tokio::test]
async fn object_commands_need_a_login(
    #[case] call: ObjectCall,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Buy milk".into())],
        )
        .start()
        .await?;
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_locked_sticker_is_not_changed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Shape("Lock".into())
                    .set("type", json!("group"))
                    .set("locked", json!(true)),
                FixtureObject::Sticker("Buy milk".into()).inside(1),
            ],
        )
        .start()
        .await?;
    let sticker = holst.fixture_object("Plans", 2)?;
    let edited = holst
        .on(surface)
        .change_object_text("Plans", &sticker, "Buy oat milk")
        .await?;
    then(&edited)
        .failed_saying("is locked on the board; unlock it in the web client first")
        .signalled_as(surface.permission_denied());
    let removed = holst.on(surface).remove_object("Plans", &sticker).await?;
    then(&removed)
        .failed_saying("is locked on the board; unlock it in the web client first")
        .signalled_as(surface.permission_denied());
    let after = holst.on(surface).objects("Plans").await?;
    then(&after)
        .lists_objects_in_any_order(&[("group", "Lock"), ("sticker", "Buy milk")]);
    Ok(())
}

fn names_the_missing_kind(found: &Result<Value, ScenarioError>) {
    assert!(
        matches!(found, Err(ScenarioError::NoObjectOfKind { .. })),
        "{found:?}"
    );
}

#[tokio::test]
async fn a_missing_kind_is_not_reported_as_a_missing_board() -> Result<(), ScenarioError>
{
    let holst = two_notes().start().await?;
    names_the_missing_kind(&holst.first_of_kind("Plans", "image"));
    Ok(())
}
