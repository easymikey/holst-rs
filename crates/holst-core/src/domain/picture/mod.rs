//! A board area drawn as SVG the way the web client draws it; no I/O.

mod arrow;
pub mod object;
mod shapes;
mod svg;
mod units;

use std::{cmp::Ordering, collections::HashMap};

use itertools::Itertools;
use serde::Serialize;

#[cfg(test)] pub(crate) use self::object::is_xml;
pub use self::units::{
    BoardLength,
    MAX_SIDE_FILE,
    MAX_SIDE_MODEL,
    PixelSide,
    PixelsPerUnit,
};
use self::{
    object::{image_address, object_svg},
    svg::{FONTS, format_number},
    units::{MAX_PIXELS, MAX_SIDE},
};
use crate::{
    domain::{
        area::{Area, scene::Scene},
        board::Object,
        geometry::Rectangle,
        palette,
    },
    error::{Result, check_all::both},
};

/// Where image files come from and which are already inlined.
#[derive(Debug, Clone, Copy)]
pub struct PictureImages<'a> {
    pub storage: &'a str,
    pub replacements: &'a HashMap<String, String>,
}

impl PictureImages<'_> {
    fn address<'b>(&'b self, address: &'b str) -> &'b str {
        self.replacements
            .get(address)
            .map_or(address, String::as_str)
    }
}

/// A rendered area: the SVG, what it covers and its pixel size.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Picture {
    #[serde(skip)]
    pub svg: String,
    pub rectangle: Rectangle,
    pub object_count: usize,
    pub width: u32,
    pub height: u32,
    pub scale: PixelsPerUnit,
    #[serde(skip)]
    pub image_addresses: Vec<String>,
}

const READABLE_SCALE: f64 = 0.1;

/// `scale` lowered, as far as needed, so that neither side of `area` exceeds
/// `max_side` pixels and the whole stays within 4096 by 4096 pixels.
fn fitted_scale(
    rectangle: &Rectangle,
    scale: PixelsPerUnit,
    max_side: PixelSide,
) -> PixelsPerUnit {
    let longest = rectangle.width.max(rectangle.height).max(1.0);
    let surface = rectangle.width.max(1.0) * rectangle.height.max(1.0);
    let side = max_side.get().floor();
    let budget = ((MAX_PIXELS - MAX_SIDE - 1.0) / surface).sqrt();
    scale.lowered_to(side / longest).lowered_to(budget)
}

/// Draw everything of `scene` that shows in `area` at `scale` pixels per
/// board unit.
fn draw_rectangle(
    scene: &Scene<'_>,
    rectangle: Rectangle,
    scale: PixelsPerUnit,
    images: &PictureImages<'_>,
) -> Picture {
    let shown = scene.within(&rectangle);
    let layered: Vec<(Vec<f64>, &Object)> = shown
        .iter()
        .map(|object| (layers(scene, object), *object))
        .sorted_by(|(a, _), (b, _)| compare_layers(a, b))
        .collect();
    let body = layered
        .iter()
        .map(|(_, object)| object_svg(scene, object, images))
        .collect::<Vec<_>>()
        .concat();
    let width = scale.pixels(rectangle.width);
    let height = scale.pixels(rectangle.height);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"{width}\" height=\"{height}\" viewBox=\"{} {} {} {}\" font-family=\"{FONTS}\">\
         <defs><filter id=\"shadow\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"150%\">\
         <feDropShadow dx=\"0\" dy=\"4\" stdDeviation=\"6\" flood-color=\"#000\" \
         flood-opacity=\"0.15\"/></filter></defs>\
         <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>{body}</svg>",
        format_number(rectangle.x),
        format_number(rectangle.y),
        format_number(rectangle.width),
        format_number(rectangle.height),
        format_number(rectangle.x),
        format_number(rectangle.y),
        format_number(rectangle.width),
        format_number(rectangle.height),
        palette::BOARD,
    );
    let images: Vec<String> = layered
        .iter()
        .filter_map(|(_, object)| image_address(object, images.storage))
        .sorted_unstable()
        .dedup()
        .collect();
    Picture {
        svg,
        rectangle,
        object_count: shown.len(),
        width,
        height,
        scale,
        image_addresses: images,
    }
}

/// What part of a board to picture and how large.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureRequest {
    pub area: Area,
    pub padding: Option<BoardLength>,
    pub scale: PixelsPerUnit,
    pub max_side: PixelSide,
}

impl PictureRequest {
    /// Returns the request when its numbers are in range; build before
    /// loading a board.
    ///
    /// # Errors
    /// [`crate::error::picture::PictureError::OutOfRange`] naming the number
    /// out of range, [`crate::error::input::InputError::Several`] when more
    /// than one is.
    pub fn new(
        area: Area,
        padding: Option<f64>,
        scale: f64,
        max_side: f64,
    ) -> Result<Self> {
        both(
            both(BoardLength::optional(padding), PixelsPerUnit::new(scale)),
            PixelSide::new(max_side),
        )
        .map(|((padding, scale), max_side)| Self {
            area,
            padding,
            scale,
            max_side,
        })
    }
}

impl Picture {
    /// A warning when the picture is too small to read, e.g. an object far away
    /// stretched the area.
    #[must_use]
    pub fn warning(&self) -> Option<String> {
        (self.scale.get() < READABLE_SCALE).then(|| {
            format!(
                "the area is {:.0}×{:.0} board units, drawn at {:.4} px per unit: text is \
                 unreadable; objects far apart stretch a whole-board picture, so pick a \
                 frame, objects or a smaller area",
                self.rectangle.width, self.rectangle.height, self.scale
            )
        })
    }
}

fn draw_area(
    scene: &Scene<'_>,
    request: &PictureRequest,
    images: &PictureImages<'_>,
) -> Result<Picture> {
    let padding = request
        .padding
        .map_or_else(|| request.area.default_padding(), BoardLength::get);
    let rectangle = scene.area_rectangle(&request.area, padding)?;
    let scale = fitted_scale(&rectangle, request.scale, request.max_side);
    Ok(draw_rectangle(scene, rectangle, scale, images))
}

/// A picture and the object a link to it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedPicture {
    pub picture: Picture,
    pub target: Option<String>,
}

/// The picture a request asks for, with no image inlined, and its link target.
///
/// # Errors
/// As [`Scene::area_rectangle`].
pub fn draw(
    objects: &[Object],
    request: &PictureRequest,
    storage: &str,
) -> Result<LinkedPicture> {
    let inlined = HashMap::new();
    let images = PictureImages {
        storage,
        replacements: &inlined,
    };
    draw_with(objects, request, &images)
}

/// Returns the picture a request asks for with `images`, and its link target.
///
/// # Errors
/// As [`Scene::area_rectangle`].
pub fn draw_with(
    objects: &[Object],
    request: &PictureRequest,
    images: &PictureImages<'_>,
) -> Result<LinkedPicture> {
    let scene = Scene::new(objects);
    let picture = draw_area(&scene, request, images)?;
    let target = scene
        .link_target(&request.area, &picture.rectangle)
        .map(|object| object.id.clone());
    Ok(LinkedPicture { picture, target })
}

fn layers(scene: &Scene<'_>, object: &Object) -> Vec<f64> {
    std::iter::successors(Some(object), |o| {
        o.parent.as_deref().and_then(|parent| scene.get(parent))
    })
    .take(16)
    .map(|o| o.z_index)
    .collect::<Vec<f64>>()
    .into_iter()
    .rev()
    .collect()
}

fn compare_layers(a: &[f64], b: &[f64]) -> Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.total_cmp(y))
        .find(|order| order.is_ne())
        .unwrap_or_else(|| a.len().cmp(&b.len()))
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

    use std::collections::HashMap;

    use serde_json::{Value, json};

    use crate::{
        domain::{
            area::{Area, scene::Scene},
            board::{Object, object_of},
            geometry::Rectangle,
            picture::{
                MAX_PIXELS,
                Picture,
                PictureImages,
                PictureRequest,
                PixelSide,
                PixelsPerUnit,
                draw_rectangle,
                draw_with,
                fitted_scale,
                is_xml,
                object::frame_colors,
                svg::{escape, format_number},
            },
        },
        error::{Error, Result, input::InputError},
    };

    fn object(fields: &Value, text: &str) -> Option<Object> {
        object_of(fields.clone()).map(|object| Object {
            text: text.to_owned(),
            ..object
        })
    }

    fn test_board() -> Vec<Object> {
        [
            object(
                &json!({"id": "s", "type": "sticker", "position": {"x": 0, "y": 0}, "width": 192,
                "height": 192, "zIndex": 8380, "fillColor": {"color": "yellow4", "opacity": 1}}),
                "edited sticker\nsecond line",
            ),
            object(
                &json!({"id": "a", "type": "arrow", "position": {"x": 0, "y": 0}, "zIndex": 13346,
                "start": {"objectId": "s", "point": {"x": 0, "y": 0}},
                "end": {"objectId": "h", "point": {"x": 0, "y": 0}}, "arrowType": "elbow",
                "strokeColor": {"color": "gray8", "opacity": 1}, "strokeWidth": 2}),
                "",
            ),
            object(
                &json!({"id": "h", "type": "shape", "shapeType": "ellipse", "position": {"x": 600, "y": 0},
                "width": 260, "height": 160, "zIndex": 12073, "fillColor": {"color": "green4", "opacity": 1},
                "strokeColor": {"color": "gray12", "opacity": 1}, "strokeWidth": 2}),
                "hud shape",
            ),
            object(
                &json!({"id": "t", "type": "simple-text", "position": {"x": 300, "y": 250}, "zIndex": 10342}),
                "hud text line one\nline two",
            ),
            object(
                &json!({"id": "f", "type": "frame", "position": {"x": 0, "y": 450}, "width": 600,
                "height": 300, "zIndex": -2316, "fillColor": {"color": "white3", "opacity": 1},
                "labelText": "hud frame edited"}),
                "hud frame edited",
            ),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn render_all(objects: &[Object]) -> Picture {
        let inlined = HashMap::new();
        let images = PictureImages {
            storage: "https://storage-edge.app.holst.so/storage/",
            replacements: &inlined,
        };
        let scene = Scene::new(objects);
        let area = scene
            .area_rectangle(&Area::Board, 40.0)
            .unwrap_or(Rectangle::new(0.0, 0.0, 1.0, 1.0));
        draw_rectangle(&scene, area, PixelsPerUnit::ONE, &images)
    }

    #[test]
    fn every_object_is_drawn_in_layer_order() {
        let picture = render_all(&test_board());
        assert_eq!(picture.object_count, 5);
        let at = |id: &str| {
            picture
                .svg
                .find(&format!("id=\"o-{id}\""))
                .unwrap_or(usize::MAX)
        };
        assert!(
            at("f") < at("s")
                && at("s") < at("t")
                && at("t") < at("h")
                && at("h") < at("a")
        );
        assert!(picture.svg.contains("fill=\"#fcf3af\""), "sticker yellow4");
        assert!(picture.svg.contains("<ellipse"), "shape kind");
        assert!(
            picture.svg.contains(">hud frame edited</tspan>"),
            "frame label"
        );
        assert!(picture.svg.contains(">second"), "sticker text");
        assert!(picture.svg.contains("<polygon"), "arrowhead");
    }

    #[test]
    fn pictures_name_their_images_and_draw_inlined_ones() -> Result<()> {
        let objects = [object(
            &json!({"id": "i", "type": "image", "position": {"x": 0, "y": 0},
                    "width": 10, "height": 10, "name": "a.png"}),
            "",
        )]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let address = "https://storage-edge.app.holst.so/storage/a.png";
        assert_eq!(render_all(&objects).image_addresses, [address]);
        let inlined = HashMap::from([(address.to_owned(), "images/a.png".to_owned())]);
        let images = PictureImages {
            storage: "https://storage-edge.app.holst.so/storage/",
            replacements: &inlined,
        };
        let request = request(1.0, 1568.0, None)?;
        let drawn = draw_with(&objects, &request, &images)?;
        assert!(drawn.picture.svg.contains("xlink:href=\"images/a.png\""));
        assert_eq!(drawn.picture.image_addresses, [address]);
        Ok(())
    }

    #[test]
    fn text_is_escaped_and_numbers_are_short() {
        assert_eq!(escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
        assert_eq!(format_number(-0.001), "0");
        assert_eq!(format_number(1.005_f64.mul_add(1.0, 0.0)), "1");
        assert_eq!(format_number(12.345), "12.35");
        let objects = [object(
            &json!({"id": "x", "type": "sticker", "position": {"x": 0, "y": 0}}),
            "<b>",
        )]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        assert!(render_all(&objects).svg.contains("&lt;b&gt;"));
    }

    #[test]
    fn frame_colours_follow_the_web_client_rules() {
        let colours = |token: &str| {
            let colours =
                frame_colors(&json!({"fillColor": {"color": token, "opacity": 1}}));
            [
                colours.border,
                colours.title,
                colours.title_border,
                colours.title_text,
            ]
        };
        assert_eq!(
            colours("white3"),
            ["#d4d4d4", "#f1f1f1", "#d4d4d4", "#202020"]
        );
        assert_eq!(
            colours("blue3"),
            ["#b7d9f8", "#e1f0ff", "#96c7f2", "#113264"]
        );
        assert_eq!(colours("blue5")[3], "#ffffff");
    }

    #[test]
    fn scale_is_lowered_to_fit_the_largest_side() {
        let area = Rectangle::new(0.0, 0.0, 4000.0, 1000.0);
        let fitted =
            |scale: f64| -> Result<f64> {
                Ok(fitted_scale(
                    &area,
                    PixelsPerUnit::new(scale)?,
                    PixelSide::new(2000.0)?,
                )
                .get())
            };
        assert!((fitted(2.0).unwrap_or_default() - 0.5).abs() < 1e-9);
        assert!((fitted(0.25).unwrap_or_default() - 0.25).abs() < 1e-9);
    }

    fn request(
        scale: f64,
        max_side: f64,
        padding: Option<f64>,
    ) -> Result<PictureRequest> {
        PictureRequest::new(Area::Board, padding, scale, max_side)
    }

    #[test]
    fn an_object_far_away_does_not_blow_up_the_picture() -> Result<()> {
        let far = object(
            &json!({"id": "far", "type": "sticker", "position": {"x": 1e9, "y": 1e9}}),
            "far",
        );
        let objects: Vec<Object> = test_board().into_iter().chain(far).collect();
        let inlined = HashMap::new();
        let images = PictureImages {
            storage: "/",
            replacements: &inlined,
        };
        [1568.0, 16_384.0].iter().try_for_each(|max_side| {
            let picture =
                draw_with(&objects, &request(1.0, *max_side, None)?, &images)?.picture;
            let (width, height) = (f64::from(picture.width), f64::from(picture.height));
            assert!(
                width <= *max_side && height <= *max_side,
                "{width}×{height}"
            );
            assert!(width * height <= MAX_PIXELS, "{width}×{height}");
            assert!(
                (picture.rectangle.width * picture.scale.get() - width).abs() <= 1.0,
                "the scale is the one drawn at: {}",
                picture.scale
            );
            assert!(picture.warning().is_some_and(|w| w.contains("far apart")));
            Ok::<_, Error>(())
        })?;
        let square = Rectangle::new(0.0, 0.0, 10_000.0, 10_000.0);
        let widest = PixelSide::new(16_384.0)?;
        let side = (square.width
            * fitted_scale(&square, PixelsPerUnit::ONE, widest).get())
        .round();
        assert!(side * side <= MAX_PIXELS, "{side}");
        let near =
            draw_with(&test_board(), &request(1.0, 1568.0, None)?, &images)?.picture;
        assert_eq!(near.warning(), None);
        Ok(())
    }

    fn problem_count(err: &Error) -> Option<usize> {
        if let Error::Input(InputError::Several { errors: problems }) = err {
            Some(problems.len())
        } else {
            None
        }
    }

    #[test]
    fn every_bad_number_of_a_request_is_reported_at_once() {
        let refused = PictureRequest::new(Area::Board, Some(-1.0), 0.0, 8.0).err();
        assert_eq!(refused.as_ref().and_then(problem_count), Some(3));
    }

    #[test]
    fn non_finite_numbers_never_reach_the_svg() {
        assert!(request(2.0, 8192.0, Some(0.0)).is_ok());
        assert_eq!(format_number(f64::INFINITY), "0");
        assert_eq!(format_number(f64::NAN), "0");
        let objects = [
            object(
                &json!({"id": "x", "type": "sticker", "position": {"x": 0, "y": 0}, "textScale": 1e307}),
                "big",
            ),
            object(
                &json!({"id": "i", "type": "image", "position": {"x": 0, "y": 0}, "width": 10,
                    "height": 10, "name": "a.png", "cropTransform": [0, 0, 1e-320, 1e-320]}),
                "",
            ),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let svg = render_all(&objects).svg;
        assert!(!svg.contains("inf") && !svg.contains("NaN"), "{svg}");
        assert!(!svg.contains("o-x"), "an absurd object is left out");
    }

    #[test]
    fn a_dotted_arrow_sets_each_attribute_once() {
        let objects = [object(
            &json!({"id": "a", "type": "arrow", "position": {"x": 0, "y": 0},
                    "strokeStyle": "dotted", "start": {"point": {"x": 0, "y": 0}},
                    "end": {"point": {"x": 100, "y": 0}}}),
            "",
        )]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let svg = render_all(&objects).svg;
        assert!(is_xml(&svg), "{svg}");
        assert_eq!(svg.matches("stroke-linecap").count(), 1, "{svg}");
    }

    fn drawing_path_of(path: &Value) -> String {
        let objects = object(
            &json!({"id": "d", "type": "drawing", "position": {"x": 0, "y": 0}, "path": path}),
            "",
        )
        .into_iter()
        .collect::<Vec<_>>();
        let svg = render_all(&objects).svg;
        let start = svg.find(" d=\"").map_or(0, |at| at + 4);
        svg.get(start..)
            .and_then(|tail| tail.split('"').next())
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn a_drawing_stored_as_a_number_list_becomes_a_polyline() {
        assert_eq!(
            drawing_path_of(&json!("0,0,0,2.5,-5.2,10.1")),
            "M0 0 L0 2.5 L-5.2 10.1"
        );
        assert_eq!(drawing_path_of(&json!([1, 2, 3, 4])), "M1 2 L3 4");
    }

    #[test]
    fn a_new_pen_stroke_draws_one_path() {
        let objects = object(
            &json!({"id": "d", "type": "drawing", "algorithm": "lazy",
                    "path": "0,0,150,50,300,100", "pivot": {"x": 0, "y": 0},
                    "position": {"x": 300, "y": 500}, "strokeWidth": 3,
                    "color": 1_973_790, "alpha": 1, "parentId": null}),
            "",
        )
        .into_iter()
        .collect::<Vec<_>>();
        let svg = render_all(&objects).svg;
        assert_eq!(svg.matches("<path").count(), 1, "{svg}");
        assert!(svg.contains("d=\"M0 0 L150 50 L300 100\""), "{svg}");
        assert!(
            svg.contains("stroke=\"#1e1e1e\" stroke-width=\"3\""),
            "{svg}"
        );
    }

    #[test]
    fn a_drawing_stored_as_path_data_is_kept() {
        assert_eq!(drawing_path_of(&json!("M0 0 L5 5 Z")), "M0 0 L5 5 Z");
    }

    #[test]
    fn control_characters_never_break_the_xml() {
        let objects = [
            object(
                &json!({"id": "s\u{1}", "type": "sticker", "position": {"x": 0, "y": 0}}),
                "first\u{b}second\u{c}third\u{0}\u{1b}\u{fffe}",
            ),
            object(
                &json!({"id": "f", "type": "frame", "position": {"x": 0, "y": 300},
                        "width": 400, "height": 100}),
                "title\u{8}\u{ffff}",
            ),
            object(
                &json!({"id": "i", "type": "image", "position": {"x": 0, "y": 500}, "width": 10,
                        "height": 10, "name": "a\u{2}.png"}),
                "",
            ),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let svg = render_all(&objects).svg;
        assert!(is_xml(&svg), "{svg}");
        assert!(svg.contains(">first</tspan>") && svg.contains(">second</tspan>"));
        assert!(svg.contains(">title</tspan>"));
        assert!(svg.contains("id=\"o-s\"") && svg.contains("a.png"));
    }
}
