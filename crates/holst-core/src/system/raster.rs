//! SVG snapshots drawn as PNG with resvg and the system fonts.

use std::{
    collections::{BTreeSet, HashMap},
    hash::BuildHasher,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg,
    usvg::{
        ImageHrefResolver,
        ImageKind,
        fontdb::{Database, ID, Source},
    },
};

use crate::{
    domain::{image, image::ImageFormat},
    error::{Error, Result, picture::PictureError},
};

static FONTS: OnceLock<Arc<Database>> = OnceLock::new();

const NAMED: [&str; 10] = [
    "Inter",
    "Helvetica Neue",
    "Segoe UI",
    "Roboto",
    "Arial",
    "Noto Sans",
    "Source Code Pro",
    "Menlo",
    "Consolas",
    "DejaVu Sans Mono",
];

fn fonts() -> Arc<Database> {
    Arc::clone(FONTS.get_or_init(|| Arc::new(system_fonts(Database::new()))))
}

fn system_fonts(mut fonts: Database) -> Database {
    fonts.load_system_fonts();
    fonts.set_sans_serif_family("Helvetica Neue");
    keep_named_in_memory(&mut fonts);
    fonts
}

fn keep_named_in_memory(fonts: &mut Database) {
    let named: BTreeSet<PathBuf> = fonts
        .faces()
        .filter(|face| {
            face.families
                .iter()
                .any(|(name, _)| NAMED.contains(&name.as_str()))
        })
        .filter_map(|face| file_of(&face.source))
        .collect();
    let read: Vec<(PathBuf, Vec<u8>)> = named
        .into_iter()
        .filter_map(|path| std::fs::read(&path).ok().map(|bytes| (path, bytes)))
        .collect();
    let replaced: Vec<ID> = fonts
        .faces()
        .filter(|face| {
            file_of(&face.source)
                .is_some_and(|path| read.iter().any(|(kept, _)| *kept == path))
        })
        .map(|face| face.id)
        .collect();
    let fonts = replaced.into_iter().fold(fonts, |fonts, id| {
        fonts.remove_face(id);
        fonts
    });
    read.into_iter().fold(fonts, |fonts, (_, bytes)| {
        fonts.load_font_data(bytes);
        fonts
    });
}

fn file_of(source: &Source) -> Option<PathBuf> {
    match source {
        Source::File(path) => Some(path.clone()),
        Source::Binary(_) => None,
    }
}

/// Draws the SVG at its own pixel size as PNG bytes, an image address found in
/// `images` drawn from its bytes; other addresses stay placeholders, nothing
/// is read from files or the network.
///
/// # Errors
/// [`crate::error::picture::PictureError::SvgParse`] for SVG resvg cannot read,
/// [`crate::error::picture::PictureError::TooLarge`] when the picture is empty
/// or too large, [`crate::error::picture::PictureError::PngEncode`].
pub fn png_with<S: BuildHasher + Sync>(
    svg: &str,
    images: &HashMap<String, Arc<Vec<u8>>, S>,
) -> Result<Vec<u8>> {
    let options = usvg::Options {
        fontdb: fonts(),
        image_href_resolver: ImageHrefResolver {
            resolve_data: ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|address, _| {
                images.get(address).and_then(|bytes| {
                    let shared = Arc::clone(bytes);
                    image::image_format(bytes).and_then(|media| match media {
                        ImageFormat::Png => Some(ImageKind::PNG(shared)),
                        ImageFormat::Jpeg => Some(ImageKind::JPEG(shared)),
                        ImageFormat::Gif => Some(ImageKind::GIF(shared)),
                        ImageFormat::Webp => Some(ImageKind::WEBP(shared)),
                        ImageFormat::Svg => None,
                    })
                })
            }),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(PictureError::SvgParse)?;
    let size = tree.size().to_int_size();
    Pixmap::new(size.width(), size.height())
        .ok_or(Error::Picture(PictureError::TooLarge {
            width: size.width(),
            height: size.height(),
        }))
        .map(|mut pixmap| {
            resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());
            pixmap
        })?
        .encode_png()
        .map_err(|_| Error::Picture(PictureError::PngEncode))
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

    use std::{collections::HashMap, sync::Arc};

    use resvg::tiny_skia::Pixmap;

    use crate::{
        error::{Error, Result, picture::PictureError},
        system::raster::png_with,
    };

    #[test]
    fn a_small_svg_becomes_a_png() -> Result<()> {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"3\">\
                   <rect width=\"4\" height=\"3\" fill=\"#fcf3af\"/></svg>";
        let bytes = png_with(svg, &HashMap::new())?;
        assert_eq!(bytes.get(1..4), Some(&b"PNG"[..]));
        assert!(matches!(
            png_with("not svg", &HashMap::new()),
            Err(Error::Picture(PictureError::SvgParse(_)))
        ));
        Ok(())
    }

    #[test]
    fn known_images_are_drawn_from_their_bytes() -> Result<()> {
        let dot = png_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">\
             <rect width=\"2\" height=\"2\" fill=\"#ff0000\"/></svg>",
            &HashMap::new(),
        )?;
        let svg = |href: &str| {
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" \
                 xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"2\" height=\"2\">\
                 <image width=\"2\" height=\"2\" xlink:href=\"{href}\"/></svg>"
            )
        };
        let images = HashMap::from([("https://s/a.png".to_owned(), Arc::new(dot))]);
        let drawn = png_with(&svg("https://s/a.png"), &images)?;
        let missing = png_with(&svg("https://s/b.png"), &images)?;
        let red = |bytes: &[u8]| {
            Pixmap::decode_png(bytes)
                .ok()
                .and_then(|pixmap| pixmap.pixel(0, 0))
                .map(|pixel| (pixel.red(), pixel.alpha()))
        };
        assert_eq!(red(&drawn), Some((255, 255)));
        assert_eq!(red(&missing), Some((0, 0)));
        Ok(())
    }
}
