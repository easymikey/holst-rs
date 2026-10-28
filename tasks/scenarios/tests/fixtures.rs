//! Fixtures several scenario files share.

use holst_scenarios::{FixtureObject, Given, given};

pub(crate) fn two_boards_with_milk() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Sticker("Buy milk".into()),
                FixtureObject::FreeText("Roadmap".into()),
            ],
        )
        .board_with_objects(
            "Groceries",
            "Team",
            vec![
                FixtureObject::Sticker("Milk".into()),
                FixtureObject::Shape("Milk run".into()),
            ],
        )
}

// A frame Q4 holding two of the four objects, the others far away.
pub(crate) fn plan_with_a_frame() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plan",
        "Team",
        vec![
            FixtureObject::Frame("Q4".into()).at(0, 0).sized(400, 300),
            FixtureObject::Sticker("In one".into()).at(50, 50).inside(1),
            FixtureObject::Sticker("In two".into())
                .at(150, 100)
                .inside(1),
            FixtureObject::Sticker("Far away".into()).at(3000, 2000),
        ],
    )
}

/// Logged in, workspace Team, board Plans holding `objects`.
pub(crate) fn plans_with(objects: Vec<FixtureObject>) -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board_with_objects("Plans", "Team", objects)
}
