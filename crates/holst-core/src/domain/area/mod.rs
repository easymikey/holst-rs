//! Where objects sit on the board: boxes, routes, areas, view; no I/O.

use crate::{
    domain::{
        area::scene::Scene,
        board::Object,
        geometry::Rectangle,
        picture::BoardLength,
    },
    error::{Error, Result, picture::PictureError},
};

pub mod route;
pub mod scene;

/// What part of the board to show or list.
#[derive(Debug, Clone, PartialEq)]
pub enum Area {
    Board,
    Rectangle(Rectangle),
    Frame(String),
    Objects(Vec<String>),
}

impl Area {
    /// At most one of an area, a frame and objects; none is the whole board.
    ///
    /// # Errors
    /// [`crate::error::picture::PictureError::SeveralAreas`] when more
    /// than one is given.
    pub fn of(
        rectangle: Option<Rectangle>,
        frame: Option<String>,
        objects: Vec<String>,
    ) -> Result<Self> {
        let objects = Some(objects).filter(|objects| !objects.is_empty());
        match (rectangle, frame, objects) {
            (None, None, None) => Ok(Area::Board),
            (Some(rectangle), None, None) => Ok(Area::Rectangle(rectangle)),
            (None, Some(frame), None) => Ok(Area::Frame(frame)),
            (None, None, Some(objects)) => Ok(Area::Objects(objects)),
            _ => Err(Error::Picture(PictureError::SeveralAreas)),
        }
    }

    /// The margin around the selection when none is asked for.
    #[must_use]
    pub const fn default_padding(&self) -> f64 {
        match self {
            Area::Board | Area::Objects(_) => 40.0,
            Area::Rectangle(_) => 0.0,
            Area::Frame(_) => 16.0,
        }
    }
}

/// The objects a selection lists, the area and what a link to it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedArea<'a> {
    pub rectangle: Option<Rectangle>,
    pub objects: Vec<&'a Object>,
    pub target: Option<&'a Object>,
}

/// Every object for the whole board, else what shows in the selection.
///
/// # Errors
/// As [`Scene::area_rectangle`].
pub fn listed_area<'a>(
    objects: &'a [Object],
    area: &Area,
    padding: Option<BoardLength>,
) -> Result<ListedArea<'a>> {
    match area {
        Area::Board => Ok(ListedArea {
            rectangle: None,
            objects: objects.iter().collect(),
            target: None,
        }),
        area @ (Area::Rectangle(_) | Area::Frame(_) | Area::Objects(_)) => {
            let scene = Scene::new(objects);
            let Covered {
                rectangle,
                objects: shown,
            } = covered(&scene, area, padding)?;
            let target = scene.link_target(area, &rectangle);
            Ok(ListedArea {
                rectangle: Some(rectangle),
                objects: shown,
                target,
            })
        }
    }
}

/// The area a selection covers and the objects in it, in reading order.
#[derive(Debug, Clone, PartialEq)]
struct Covered<'a> {
    rectangle: Rectangle,
    objects: Vec<&'a Object>,
}

fn covered<'a>(
    scene: &Scene<'a>,
    area: &Area,
    padding: Option<BoardLength>,
) -> Result<Covered<'a>> {
    let padding = padding.map_or_else(|| area.default_padding(), BoardLength::get);
    let rectangle = scene.area_rectangle(area, padding)?;
    Ok(Covered {
        objects: scene.within(&rectangle),
        rectangle,
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

    use serde_json::{Value, json};

    use crate::{
        domain::{
            area::{
                Area,
                listed_area,
                scene::{FRAME_TITLE_BAND, Scene},
            },
            board::{ArrowEnds, DrawnArrowEnds, Object, ObjectKind},
            geometry::{Point, Rectangle, StoredPoint},
        },
        error::{Error, Result, lookup, lookup::LookupError, picture::PictureError},
    };

    fn object(fields: &Value) -> Object {
        let float = |name: &str| fields.get(name).and_then(Value::as_f64);
        Object {
            id: fields["id"].as_str().unwrap_or_default().to_owned(),
            kind: ObjectKind::of(fields["type"].as_str().unwrap_or_default()),
            parent: fields
                .get("parentId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            x: fields
                .pointer("/position/x")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
            y: fields
                .pointer("/position/y")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
            width: float("width"),
            height: float("height"),
            z_index: float("zIndex").unwrap_or_default(),
            text: fields
                .get("labelText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            color: fields
                .pointer("/fillColor/color")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ends: None,
            author: None,
            file: crate::domain::board::stored_file(
                &ObjectKind::of(fields["type"].as_str().unwrap_or_default()),
                fields,
            ),
            fields: fields.clone(),
        }
    }

    fn board() -> Vec<Object> {
        [
            json!({"id": "s1", "type": "sticker", "position": {"x": 0, "y": 0}, "width": 192, "height": 192}),
            json!({"id": "h1", "type": "shape", "position": {"x": 600, "y": 0}, "width": 260, "height": 160}),
            json!({"id": "a1", "type": "arrow", "position": {"x": 0, "y": 0},
                   "start": {"objectId": "s1", "point": {"x": 0, "y": 0}},
                   "end": {"objectId": "h1", "point": {"x": 0, "y": 0}}, "arrowType": "elbow"}),
            json!({"id": "f1", "type": "frame", "position": {"x": 0, "y": 450}, "width": 600, "height": 300, "labelText": "Plan"}),
            json!({"id": "f2", "type": "frame", "position": {"x": 2000, "y": 0}, "width": 100, "height": 100, "labelText": "Plan B"}),
        ]
        .iter()
        .map(object)
        .collect()
    }

    #[test]
    fn elbow_arrow_leaves_sides_facing_each_other() {
        let objects = board();
        let scene = Scene::new(&objects);
        let route = scene.get("a1").and_then(|arrow| scene.route(arrow));
        assert_eq!(
            route.map(|r| r.points),
            Some(vec![
                StoredPoint::new(192.0, 96.0),
                StoredPoint::new(396.0, 96.0),
                StoredPoint::new(396.0, 80.0),
                StoredPoint::new(600.0, 80.0)
            ])
        );
    }

    #[test]
    fn an_arrow_is_placed_where_its_route_starts() {
        let objects = board();
        let scene = Scene::new(&objects);
        let at = |id: &str| scene.get(id).map(|object| scene.point(object));
        assert_eq!(at("a1"), Some(Point::whole(192, 96)));
        assert_eq!(at("h1"), Some(Point::whole(600, 0)));
        let ends = scene.get("a1").and_then(|arrow| scene.drawn_ends(arrow));
        assert_eq!(ends.map(|ends| ends.end), Some(Point::whole(600, 80)));
        assert_eq!(scene.get("s1").and_then(|s1| scene.drawn_ends(s1)), None);
    }

    #[test]
    fn an_arrow_without_fields_is_placed_and_found_by_its_kept_ends() {
        let drawn = DrawnArrowEnds {
            start: Point::whole(120, 340),
            end: Point::whole(400, 340),
        };
        let arrow = Object {
            ends: Some(ArrowEnds {
                start: Some("s1".into()),
                end: None,
                drawn: Some(drawn),
            }),
            fields: Value::Null,
            ..object(&json!({"id": "a1", "type": "arrow"}))
        };
        let objects = [arrow];
        let scene = Scene::new(&objects);
        let [placed] = &objects;
        assert_eq!(scene.point(placed), drawn.start);
        let ids = |area| {
            scene
                .within(&area)
                .iter()
                .map(|o| o.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Rectangle::new(100.0, 300.0, 50.0, 50.0)), ["a1"]);
        assert!(ids(Rectangle::new(0.0, 0.0, 50.0, 50.0)).is_empty());
    }

    #[test]
    fn frames_resolve_by_title_or_id_with_their_label() -> Result<()> {
        let objects = board();
        let scene = Scene::new(&objects);
        assert_eq!(scene.frame("plan")?.id, "f1", "the exact title wins");
        assert_eq!(scene.frame("f2")?.id, "f2");
        assert!(matches!(
            scene.frame("nothing"),
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Frame,
                miss: lookup::LookupMiss::NotFound { .. },
                ..
            }))
        ));
        assert_eq!(
            scene.area_rectangle(&Area::Frame("Plan".into()), 0.0)?,
            Rectangle::new(
                0.0,
                450.0 - FRAME_TITLE_BAND,
                600.0,
                300.0 + FRAME_TITLE_BAND
            )
        );
        Ok(())
    }

    #[test]
    fn short_prefixes_pick_no_frame_and_categories_never_outrank_each_other()
    -> Result<()> {
        let frame = |id: &str, title: &str| {
            object(
                &json!({"id": id, "type": "frame", "position": {"x": 0, "y": 0},
                "width": 10, "height": 10, "labelText": title}),
            )
        };
        let backlog = [frame("1f00aa", "Backlog"), frame("c200bb", "Retro")];
        let scene = Scene::new(&backlog);
        assert!(
            matches!(
                scene.frame("1"),
                Err(Error::Lookup(LookupError {
                    miss: lookup::LookupMiss::NotFound { .. },
                    ..
                }))
            ),
            "a one-letter id start picked a frame"
        );
        let both = [
            frame("abcdef01-0000-4000-8000-000000000003", "abcdef01 again"),
            frame("12345678-0000-4000-8000-000000000002", "abcdef01 notes"),
            frame("abcdef01-0000-4000-8000-000000000001", "Plan"),
        ];
        let listed = |objects: &[Object]| {
            Scene::new(objects)
                .frame("abcdef01")
                .err()
                .map(|error| format!("{error:?}"))
        };
        let expected = [
            ("Plan", "abcdef01-0000-4000-8000-000000000001"),
            ("abcdef01 again", "abcdef01-0000-4000-8000-000000000003"),
            ("abcdef01 notes", "12345678-0000-4000-8000-000000000002"),
        ]
        .map(|(name, id)| lookup::Candidate {
            name: name.to_owned(),
            id: Some(id.to_owned()),
            ..lookup::Candidate::default()
        });
        let ambiguous = format!(
            "{:?}",
            Error::Lookup(LookupError {
                kind: lookup::LookupKind::Frame,
                reference: "abcdef01".to_owned(),
                miss: lookup::LookupMiss::Ambiguous {
                    candidates: expected.into_iter().collect(),
                    unread: lookup::SkippedWorkspaces::default(),
                },
            })
        );
        assert_eq!(listed(&both), Some(ambiguous.clone()));
        let reversed: Vec<Object> = both.iter().rev().cloned().collect();
        assert_eq!(listed(&reversed), Some(ambiguous), "order follows input");
        let prefix_only = [frame("abcdef01-0000-4000-8000-000000000001", "Plan")];
        assert_eq!(
            Scene::new(&prefix_only).frame("ABCDEF01")?.text,
            "Plan",
            "a long prefix alone picks"
        );
        Ok(())
    }

    #[test]
    fn a_frame_title_beats_an_id_prefix() -> Result<()> {
        let frame = |id: &str, title: &str| {
            object(
                &json!({"id": id, "type": "frame", "position": {"x": 0, "y": 0},
                "width": 10, "height": 10, "labelText": title}),
            )
        };
        let objects = [
            frame("2f0c", "Backlog"),
            frame("9a1b", "2"),
            frame("7c2d", "Plan 7"),
            frame("8d3e", "Plan 8"),
            frame("5e4f", "Retro"),
            frame("5f50", "Retro"),
        ];
        let scene = Scene::new(&objects);
        let found = |reference: &str| scene.frame(reference).map(|o| o.id.clone());
        assert_eq!(found("2")?, "9a1b", "exact title before id prefix");
        assert_eq!(found("2F0C")?, "2f0c", "exact id before everything");
        assert_eq!(
            found("7")?,
            "7c2d",
            "a short id start is no prefix: the title Plan 7 holds the 7"
        );
        assert_eq!(found("plan 8")?, "8d3e");
        assert_eq!(found("back")?, "2f0c", "unique part of a title");
        assert!(matches!(
            found("retro"),
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Frame,
                miss: lookup::LookupMiss::Ambiguous { .. },
                ..
            }))
        ));
        assert!(matches!(
            found("plan"),
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Frame,
                miss: lookup::LookupMiss::Ambiguous { .. },
                ..
            }))
        ));
        assert!(matches!(
            found("zz"),
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Frame,
                miss: lookup::LookupMiss::NotFound { .. },
                ..
            }))
        ));
        Ok(())
    }

    #[test]
    fn an_arrow_to_a_missing_object_is_left_out() -> Result<()> {
        let objects: Vec<Object> = [
            json!({"id": "s1", "type": "sticker", "position": {"x": 1000, "y": 1000}, "width": 192, "height": 192}),
            json!({"id": "g", "type": "group", "position": {"x": 0, "y": 0}}),
            json!({"id": "gone", "type": "arrow", "position": {"x": 0, "y": 0},
                   "start": {"objectId": "s1", "point": {"x": 0, "y": 0}},
                   "end": {"objectId": "deleted", "point": {"x": 0, "y": 0}}}),
            json!({"id": "grouped", "type": "arrow", "position": {"x": 0, "y": 0},
                   "start": {"objectId": "g", "point": {"x": 0, "y": 0}},
                   "end": {"objectId": "s1", "point": {"x": 0, "y": 0}}}),
            json!({"id": "free", "type": "arrow", "position": {"x": 0, "y": 0},
                   "start": {"objectId": "s1", "point": {"x": 0, "y": 0}},
                   "end": {"point": {"x": 1500, "y": 1100}}}),
        ]
        .iter()
        .map(object)
        .collect();
        let scene = Scene::new(&objects);
        let routed = |id: &str| scene.get(id).and_then(|arrow| scene.route(arrow));
        assert_eq!(routed("gone"), None);
        assert_eq!(routed("grouped"), None);
        assert!(routed("free").is_some(), "a free end stays at its point");
        let area = scene.area_rectangle(&Area::Board, 0.0)?;
        assert!(area.x >= 1000.0 && area.y >= 990.0, "{area}");
        Ok(())
    }

    #[test]
    fn absurd_numbers_keep_an_object_out_of_the_area() -> Result<()> {
        let objects: Vec<Object> = [
            json!({"id": "s1", "type": "sticker", "position": {"x": 0, "y": 0}, "width": 192, "height": 192}),
            json!({"id": "big", "type": "sticker", "position": {"x": 0, "y": 0}, "width": 192,
                   "height": 192, "textScale": 1e307}),
            json!({"id": "far", "type": "shape", "position": {"x": 1e300, "y": 0}}),
            json!({"id": "a", "type": "arrow", "position": {"x": 0, "y": 0}, "strokeWidth": 1e308,
                   "start": {"point": {"x": 0, "y": 0}}, "end": {"point": {"x": 10, "y": 0}}}),
        ]
        .iter()
        .map(object)
        .collect();
        let scene = Scene::new(&objects);
        assert_eq!(
            scene.area_rectangle(&Area::Board, 0.0)?,
            Rectangle::new(0.0, 0.0, 192.0, 192.0)
        );
        Ok(())
    }

    #[test]
    fn a_selected_group_covers_its_members() -> Result<()> {
        let objects: Vec<Object> = [
            json!({"id": "g", "type": "group", "position": {"x": 0, "y": 0}}),
            json!({"id": "a", "type": "sticker", "parentId": "g", "position": {"x": 100, "y": 100}, "width": 192, "height": 192}),
            json!({"id": "b", "type": "sticker", "parentId": "g", "position": {"x": 400, "y": 100}, "width": 192, "height": 192}),
            json!({"id": "far", "type": "sticker", "position": {"x": 5000, "y": 5000}, "width": 192, "height": 192}),
            json!({"id": "empty", "type": "group", "position": {"x": 0, "y": 0}}),
        ]
        .iter()
        .map(object)
        .collect();
        let scene = Scene::new(&objects);
        let group = Area::Objects(vec!["g".into()]);
        assert_eq!(
            scene.area_rectangle(&group, 0.0)?,
            Rectangle::new(100.0, 100.0, 492.0, 192.0)
        );
        let padding = crate::domain::picture::BoardLength::new(0.0)?;
        let shown = listed_area(&objects, &group, Some(padding))?.objects;
        assert_eq!(
            shown
                .iter()
                .map(|shown| shown.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(matches!(
            scene.area_rectangle(&Area::Objects(vec!["empty".into()]), 0.0),
            Err(Error::Picture(PictureError::EmptyArea { .. }))
        ));
        Ok(())
    }

    #[test]
    fn a_selected_frame_links_to_itself_not_the_frame_around_it() -> Result<()> {
        let objects: Vec<Object> = board()
            .into_iter()
            .chain([object(
                &json!({"id": "outer", "type": "frame", "position": {"x": -100, "y": 300},
            "width": 900, "height": 600, "labelText": "Outer"}),
            )])
            .collect();
        let scene = Scene::new(&objects);
        let area = Area::Frame("Plan".into());
        let rectangle = scene.area_rectangle(&area, area.default_padding())?;
        let id = |found: Option<&Object>| found.map(|o| o.id.clone());
        assert_eq!(
            id(scene.rectangle_link_target(&rectangle)),
            Some("outer".into()),
            "the padded area"
        );
        assert_eq!(id(scene.link_target(&area, &rectangle)), Some("f1".into()));
        let exact = Rectangle::new(0.0, 450.0, 600.0, 300.0);
        assert_eq!(id(scene.rectangle_link_target(&exact)), Some("f1".into()));
        let area_selection = Area::Rectangle(exact);
        assert_eq!(
            id(scene.link_target(&area_selection, &exact)),
            Some("f1".into())
        );
        Ok(())
    }

    #[test]
    fn an_area_holds_what_overlaps_it() -> Result<()> {
        let objects = board();
        let scene = Scene::new(&objects);
        let ids = |rectangle: Rectangle| {
            scene
                .within(&rectangle)
                .iter()
                .map(|o| o.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Rectangle::new(-10.0, -10.0, 50.0, 50.0)), ["s1"]);
        assert_eq!(ids(Rectangle::new(300.0, 90.0, 10.0, 10.0)), ["a1"]);
        assert_eq!(
            ids(Rectangle::new(1000.0, 1000.0, 5.0, 5.0)),
            Vec::<String>::new()
        );
        let around = scene
            .area_rectangle(&Area::Objects(vec!["s1".into(), "h1".into()]), 10.0)?;
        assert_eq!(around, Rectangle::new(-10.0, -10.0, 880.0, 212.0));
        assert!(matches!(
            Scene::new(&[]).area_rectangle(&Area::Board, 0.0),
            Err(Error::Picture(PictureError::EmptyBoard))
        ));
        Ok(())
    }

    #[test]
    fn sticker_box_scales_and_rotation_widens_bounds() {
        let sticker = object(
            &json!({"id": "s", "type": "sticker", "position": {"x": 10, "y": 10},
            "width": 192, "height": 192, "textScale": 2, "rotation": 90}),
        );
        let scene = Scene::new(std::slice::from_ref(&sticker));
        assert_eq!(
            scene.rectangle_of(&sticker),
            Some(Rectangle::new(10.0, 10.0, 384.0, 384.0))
        );
        let bounds = scene
            .covered_by(&sticker)
            .map(|r| (r.x.round(), r.y.round(), r.width.round()));
        assert_eq!(bounds, Some((-374.0, 10.0, 384.0)));
    }

    #[test]
    fn a_frame_without_a_size_measures_as_a_new_frame() {
        let frame = object(
            &json!({"id": "f", "type": "frame", "position": {"x": 10, "y": 20}}),
        );
        let scene = Scene::new(std::slice::from_ref(&frame));
        assert_eq!(
            scene.rectangle_of(&frame),
            Some(Rectangle::new(10.0, 20.0, 800.0, 600.0))
        );
        assert_eq!(
            scene.covered_by(&frame),
            Some(Rectangle::new(
                10.0,
                20.0 - FRAME_TITLE_BAND,
                800.0,
                600.0 + FRAME_TITLE_BAND
            ))
        );
        let shape =
            object(&json!({"id": "h", "type": "shape", "position": {"x": 0, "y": 0}}));
        assert_eq!(
            Scene::new(std::slice::from_ref(&shape)).rectangle_of(&shape),
            Some(Rectangle::new(0.0, 0.0, 200.0, 120.0))
        );
    }

    #[test]
    fn table_cells_sit_in_their_line_and_column() {
        let objects: Vec<Object> = [
            json!({"id": "t", "type": "table", "position": {"x": 100, "y": 100},
                   "column-a": {"index": 0, "width": 200}, "column-b": {"index": 1, "width": 300},
                   "row-r": {"index": 0, "height": 50}, "row-q": {"index": 1, "height": 70}}),
            json!({"id": "c", "type": "table-cell", "parentId": "t", "columnId": "b", "rowId": "q"}),
        ]
        .iter()
        .map(object)
        .collect();
        let scene = Scene::new(&objects);
        let table = scene.get("t").and_then(|t| scene.rectangle_of(t));
        assert_eq!(table, Some(Rectangle::new(100.0, 100.0, 500.0, 120.0)));
        let cell = scene.get("c").and_then(|c| scene.rectangle_of(c));
        assert_eq!(cell, Some(Rectangle::new(300.0, 150.0, 300.0, 70.0)));
    }
}
