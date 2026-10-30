//! Image bytes told by their first bytes, and `data:` URLs of them; no I/O.

use std::iter;

use base64::Engine as _;

/// An image format a picture can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
}

impl ImageFormat {
    /// Returns the MIME type, e.g. `image/png`.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Webp => "image/webp",
            ImageFormat::Svg => "image/svg+xml",
        }
    }
}

/// Returns the image format of `bytes` by their signature; the storage's
/// `Content-Type` is not trusted.
///
/// ```
/// use holst_core::domain::image::{ImageFormat, image_format};
///
/// assert_eq!(
///     image_format(b"\x89PNG\r\n\x1a\n...."),
///     Some(ImageFormat::Png)
/// );
/// assert_eq!(image_format(b"\xff\xd8\xff\xe0"), Some(ImageFormat::Jpeg));
/// assert_eq!(
///     image_format(b"RIFF\0\0\0\0WEBPVP8 "),
///     Some(ImageFormat::Webp)
/// );
/// assert_eq!(image_format(b"PK\x03\x04"), None);
/// ```
#[must_use]
pub fn image_format(bytes: &[u8]) -> Option<ImageFormat> {
    let svg = || is_svg(bytes);
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xff, 0xd8, 0xff, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        [b'R', b'I', b'F', b'F', ..] if bytes.get(8..12) == Some(b"WEBP") => {
            Some(ImageFormat::Webp)
        }
        _ => svg().then_some(ImageFormat::Svg),
    }
}

/// Where a `<!DOCTYPE` stands; a `>` in quotes or in `[...]` does not end it.
#[derive(Clone, Copy)]
enum DoctypeState {
    Open,
    Quoted(u8),
    Subset,
    SubsetQuoted(u8),
    Closed,
}

impl DoctypeState {
    const fn next(self, byte: u8) -> Self {
        match (self, byte) {
            (Self::Open, b'>') => Self::Closed,
            (Self::Open, b'"' | b'\'') => Self::Quoted(byte),
            (Self::Open, b'[') => Self::Subset,
            (Self::Quoted(quote), _) if quote == byte => Self::Open,
            (Self::Subset, b']') => Self::Open,
            (Self::Subset, b'"' | b'\'') => Self::SubsetQuoted(byte),
            (Self::SubsetQuoted(quote), _) if quote == byte => Self::Subset,
            (state, _) => state,
        }
    }
}

/// Returns what follows the first `end` in `bytes`.
fn after<'a>(bytes: &'a [u8], end: &[u8]) -> Option<&'a [u8]> {
    bytes
        .windows(end.len())
        .position(|window| window == end)
        .and_then(|at| bytes.get(at + end.len()..))
}

/// Returns what follows the declaration whose `<!` has been read.
fn after_doctype(bytes: &[u8]) -> Option<&[u8]> {
    bytes
        .iter()
        .scan(DoctypeState::Open, |state, &byte| {
            *state = state.next(byte);
            Some(*state)
        })
        .position(|state| matches!(state, DoctypeState::Closed))
        .and_then(|at| bytes.get(at + 1..))
}

/// Returns what follows one prolog item (`<?..?>`, comment, doctype) at the
/// start of `bytes`, or `None` when the next thing is not one.
fn after_prolog_item(bytes: &[u8]) -> Option<&[u8]> {
    match bytes.trim_ascii_start() {
        [b'<', b'?', rest @ ..] => after(rest, b"?>"),
        [b'<', b'!', b'-', b'-', rest @ ..] => after(rest, b"-->"),
        [b'<', b'!', rest @ ..] => after_doctype(rest),
        _ => None,
    }
}

/// Whether the root element of the document in `bytes` is `svg`.
fn is_svg(bytes: &[u8]) -> bool {
    let document = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    iter::successors(Some(document), |rest| after_prolog_item(rest))
        .last()
        .map(<[u8]>::trim_ascii_start)
        .and_then(|root| root.strip_prefix(b"<svg"))
        .is_some_and(|tail| {
            matches!(
                tail.first(),
                None | Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')
            )
        })
}

/// Returns `bytes` as a `data:` URL when they are an image.
///
/// ```
/// use holst_core::domain::image::data_url;
///
/// assert_eq!(
///     data_url(b"GIF89a").as_deref(),
///     Some("data:image/gif;base64,R0lGODlh")
/// );
/// assert_eq!(data_url(b"text"), None);
/// ```
#[must_use]
pub fn data_url(bytes: &[u8]) -> Option<String> {
    image_format(bytes).map(|media| {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!("data:{};base64,{encoded}", media.mime())
    })
}

/// Returns the pixel size `(width, height)` read from the header of `bytes`;
/// `None` for a truncated header, an unknown format or SVG.
///
/// ```
/// use holst_core::domain::image::pixel_size;
///
/// assert_eq!(pixel_size(b"GIF89a\x10\0\x20\0"), Some((16, 32)));
/// assert_eq!(pixel_size(b"text"), None);
/// ```
#[must_use]
pub fn pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = match image_format(bytes)? {
        ImageFormat::Png => png_size(bytes),
        ImageFormat::Gif => gif_size(bytes),
        ImageFormat::Jpeg => jpeg_size(bytes, 2),
        ImageFormat::Webp => webp_size(bytes),
        ImageFormat::Svg => None,
    }?;
    (size.0 > 0 && size.1 > 0).then_some(size)
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
    Some(u32::from(u16::from_be_bytes(pair)))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
    Some(u32::from(u16::from_le_bytes(pair)))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let triple = bytes.get(at..at + 3)?;
    Some(
        triple
            .iter()
            .rev()
            .fold(0, |sum, &byte| sum << 8 | u32::from(byte)),
    )
}

/// The `IHDR` chunk comes first: 4-byte width and height after its tag.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    bytes.get(12..16).filter(|tag| *tag == b"IHDR")?;
    let width: [u8; 4] = bytes.get(16..20)?.try_into().ok()?;
    let height: [u8; 4] = bytes.get(20..24)?.try_into().ok()?;
    Some((u32::from_be_bytes(width), u32::from_be_bytes(height)))
}

/// The logical screen descriptor follows the 6-byte signature.
fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((le16(bytes, 6)?, le16(bytes, 8)?))
}

/// Where the walk over the marker segments of a JPEG goes next.
enum JpegStep {
    Next(usize),
    Size((u32, u32)),
}

/// Walks the marker segments from `at` up to the first start-of-frame one.
fn jpeg_size(bytes: &[u8], at: usize) -> Option<(u32, u32)> {
    let last = iter::successors(Some(JpegStep::Next(at)), |step| match step {
        JpegStep::Next(from) => jpeg_step(bytes, *from),
        JpegStep::Size(_) => None,
    })
    .last();
    match last {
        Some(JpegStep::Size(size)) => Some(size),
        Some(JpegStep::Next(_)) | None => None,
    }
}

/// The marker segment at `at`; `None` when the walk cannot go on.
fn jpeg_step(bytes: &[u8], at: usize) -> Option<JpegStep> {
    bytes.get(at).filter(|prefix| **prefix == 0xff)?;
    let marker = *bytes.get(at + 1)?;
    match marker {
        0xff => Some(JpegStep::Next(at + 1)),
        0x01 | 0xd0..=0xd8 => Some(JpegStep::Next(at + 2)),
        // End of image or start of scan before any frame header.
        0xd9 | 0xda => None,
        0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
            Some(JpegStep::Size((be16(bytes, at + 7)?, be16(bytes, at + 5)?)))
        }
        _ => Some(JpegStep::Next(
            at + 2 + usize::try_from(be16(bytes, at + 2)?).ok()?,
        )),
    }
}

/// The lossy `VP8 `: a 3-byte frame tag, the start code, 14-bit sizes.
fn webp_lossy_size(bytes: &[u8]) -> Option<(u32, u32)> {
    bytes
        .get(23..26)
        .filter(|code| *code == [0x9d, 0x01, 0x2a])?;
    Some((le16(bytes, 26)? & 0x3fff, le16(bytes, 28)? & 0x3fff))
}

/// `VP8L`: a signature byte, then two 14-bit fields holding size - 1.
fn webp_lossless_size(bytes: &[u8]) -> Option<(u32, u32)> {
    bytes.get(20).filter(|signature| **signature == 0x2f)?;
    let packed: [u8; 4] = bytes.get(21..25)?.try_into().ok()?;
    let bits = u32::from_le_bytes(packed);
    Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
}

/// The first chunk after `RIFF....WEBP` is `VP8 `, `VP8L` or `VP8X`.
fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => webp_lossy_size(bytes),
        b"VP8L" => webp_lossless_size(bytes),
        b"VP8X" => Some((le24(bytes, 24)? + 1, le24(bytes, 27)? + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::domain::image::{ImageFormat, image_format, pixel_size};

    // Illustrator export, 700+ bytes before `<svg`.
    const ILLUSTRATOR: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- Generator: Adobe Illustrator 27.0.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\n\t<!ENTITY ns_extend \"http://ns.adobe.com/Extensibility/1.0/\">\n\t<!ENTITY ns_ai \"http://ns.adobe.com/AdobeIllustrator/10.0/\">\n\t<!ENTITY ns_graphs \"http://ns.adobe.com/Graphs/1.0/\">\n\t<!ENTITY ns_vars \"http://ns.adobe.com/Variables/1.0/\">\n\t<!ENTITY ns_imrep \"http://ns.adobe.com/ImageReplacement/1.0/\">\n\t<!ENTITY ns_sfw \"http://ns.adobe.com/SaveForWeb/1.0/\">\n]>\n<svg version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\"/>";

    #[rstest]
    #[case::bare(b"<svg xmlns='http://www.w3.org/2000/svg'/>", Some(ImageFormat::Svg))]
    #[case::xml_header(b"<?xml version=\"1.0\"?><svg/>", Some(ImageFormat::Svg))]
    #[case::bom(b"\xef\xbb\xbf<svg/>", Some(ImageFormat::Svg))]
    #[case::doctype_and_comment(
        b"<!-- Generator --><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"><svg/>",
        Some(ImageFormat::Svg)
    )]
    #[case::html(b"<html><body>hi</body></html>", None)]
    #[case::html_with_inline_svg(
        b"<!DOCTYPE html><html><body><svg/></body></html>",
        None
    )]
    #[case::html_svg_close(b"<html><svg/></html>", None)]
    #[case::unfinished_comment(b"<!-- <svg/>", None)]
    #[case::illustrator_prolog(ILLUSTRATOR, Some(ImageFormat::Svg))]
    fn svg_is_found_after_its_prolog(
        #[case] bytes: &[u8],
        #[case] expected: Option<ImageFormat>,
    ) {
        assert_eq!(image_format(bytes), expected);
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend(b"\x08\x06\0\0\0");
        bytes
    }

    fn cut(mut bytes: Vec<u8>, len: usize) -> Vec<u8> {
        bytes.truncate(len);
        bytes
    }

    fn jpeg(frame_marker: u8) -> Vec<u8> {
        // SOI, APP0 (length 16), then the frame header: 40 high, 30 wide.
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        bytes.extend([0; 14]);
        bytes.extend([0xff, frame_marker, 0x00, 0x11, 0x08, 0, 40, 0, 30]);
        bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    fn riff(chunk: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend(chunk);
        bytes.extend([0; 4]);
        bytes.extend(payload);
        bytes
    }

    fn webp_lossy() -> Vec<u8> {
        // Frame tag (3), start code, 300 x 200.
        riff(b"VP8 ", b"\0\0\0\x9d\x01\x2a\x2c\x01\xc8\x00")
    }

    fn webp_lossless() -> Vec<u8> {
        // 300 x 200 as (299, 199) in two 14-bit fields.
        let bits: u32 = 0x12b | (0xc7 << 14);
        let mut payload = vec![0x2f];
        payload.extend(bits.to_le_bytes());
        riff(b"VP8L", &payload)
    }

    fn webp_extended() -> Vec<u8> {
        // Flags (4), then canvas (299, 199) as 24-bit fields.
        riff(b"VP8X", b"\0\0\0\0\x2b\x01\0\xc7\0\0")
    }

    #[rstest]
    #[case::png(png(16, 16), Some((16, 16)))]
    #[case::png_wide(png(1920, 1080), Some((1920, 1080)))]
    #[case::gif(b"GIF89a\x10\0\x0a\0\x80\0\0".to_vec(), Some((16, 10)))]
    #[case::jpeg_baseline(jpeg(0xc0), Some((30, 40)))]
    #[case::jpeg_progressive(jpeg(0xc2), Some((30, 40)))]
    #[case::webp_lossy(webp_lossy(), Some((300, 200)))]
    #[case::webp_lossless(webp_lossless(), Some((300, 200)))]
    #[case::webp_extended(webp_extended(), Some((300, 200)))]
    #[case::png_truncated(cut(png(16, 16), 20), None)]
    #[case::gif_truncated(b"GIF89a\x10".to_vec(), None)]
    #[case::jpeg_truncated(cut(jpeg(0xc0), 22), None)]
    #[case::jpeg_no_frame(vec![0xff, 0xd8, 0xff, 0xda, 0, 2], None)]
    #[case::jpeg_run_of_fill_bytes([vec![0xff, 0xd8], vec![0xff; 1_000_000]].concat(), None)]
    #[case::webp_truncated(cut(webp_lossless(), 24), None)]
    #[case::svg(b"<svg width=\"10\" height=\"10\"/>".to_vec(), None)]
    #[case::unknown(b"PK\x03\x04 zip".to_vec(), None)]
    #[case::empty(Vec::new(), None)]
    fn pixel_size_is_read_from_the_header(
        #[case] bytes: Vec<u8>,
        #[case] expected: Option<(u32, u32)>,
    ) {
        assert_eq!(pixel_size(&bytes), expected);
    }
}
