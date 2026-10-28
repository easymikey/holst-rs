use std::collections::HashMap;

use serde_json::{Value, json};

use super::step;

fn sticker(x: f64, layer: f64) -> Value {
    json!({
        "type": "sticker", "position": { "x": x, "y": 0 },
        "width": 200, "height": 200, "zIndex": layer,
    })
}

/// A at 1, B at 2 and C at 4 overlap; F at 3 is far from all of them.
fn states() -> HashMap<String, Value> {
    HashMap::from([
        ("a".to_owned(), sticker(0.0, 1.0)),
        ("b".to_owned(), sticker(100.0, 2.0)),
        ("c".to_owned(), sticker(150.0, 4.0)),
        ("f".to_owned(), sticker(5000.0, 3.0)),
    ])
}

fn near(layer: Option<f64>, wanted: f64) -> bool {
    layer.is_some_and(|layer| (layer - wanted).abs() < 1e-9)
}

#[test]
fn forward_goes_between_the_next_overlapping_object_and_the_one_after() {
    assert!(near(step(&states(), "a", true), 2.0 + 2.0 / 3.0));
}

#[test]
fn backward_goes_between_the_next_overlapping_object_and_the_one_after() {
    assert!(near(step(&states(), "c", false), 1.0 + 1.0 / 3.0));
}

#[test]
fn the_top_of_its_overlaps_has_no_step_forward() {
    assert_eq!(step(&states(), "c", true), None);
}

#[test]
fn the_bottom_of_its_overlaps_has_no_step_backward() {
    assert_eq!(step(&states(), "a", false), None);
}

#[test]
fn an_object_of_another_parent_is_not_stepped_past() {
    let mut other = states();
    other.insert(
        "b".to_owned(),
        json!({ "parentId": "p", "type": "sticker", "position": { "x": 100, "y": 0 },
                "width": 200, "height": 200, "zIndex": 2 }),
    );
    let layer = step(&other, "a", true);
    assert!(layer.is_some_and(|layer| layer > 4.0), "{layer:?}");
}

#[test]
fn a_member_of_a_group_that_ignores_layers_does_not_step() {
    let mut inside = states();
    inside.insert(
        "g".to_owned(),
        json!({ "type": "group", "ignoreZIndex": true, "zIndex": 0 }),
    );
    inside.insert(
        "a".to_owned(),
        json!({ "parentId": "g", "type": "sticker", "position": { "x": 0, "y": 0 },
                "width": 200, "height": 200, "zIndex": 1 }),
    );
    assert_eq!(step(&inside, "a", true), None);
}
