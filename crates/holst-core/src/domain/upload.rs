//! What an upload must know about a local file before it goes out; no I/O.

use crate::domain::image::{ImageFormat, image_format};

const DOCX: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const PPTX: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";

/// The MIME type for the extension of file `name`, case ignored;
/// `application/octet-stream` when the extension is unknown.
///
/// ```
/// use holst_core::domain::upload::file_type;
///
/// assert_eq!(file_type("zz.PDF"), "application/pdf");
/// assert_eq!(file_type("zz.bin"), "application/octet-stream");
/// ```
#[must_use]
pub fn file_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
    match extension.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "docx" => DOCX,
        "pptx" => PPTX,
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// What an upload becomes on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadKind {
    /// A picture of this format.
    Image(ImageFormat),
    /// Any other file, or an image given as a file.
    File,
}

impl UploadKind {
    /// Returns the kind of `bytes` asked as an image (`as_image`) or a file;
    /// `None` when a non-image is asked as an image.
    ///
    /// ```
    /// use holst_core::domain::{image::ImageFormat, upload::UploadKind};
    ///
    /// let gif = b"GIF89a";
    /// assert_eq!(
    ///     UploadKind::of(gif, true),
    ///     Some(UploadKind::Image(ImageFormat::Gif))
    /// );
    /// assert_eq!(UploadKind::of(gif, false), Some(UploadKind::File));
    /// assert_eq!(UploadKind::of(b"text", true), None);
    /// ```
    #[must_use]
    pub fn of(bytes: &[u8], as_image: bool) -> Option<Self> {
        if as_image {
            image_format(bytes).map(Self::Image)
        } else {
            Some(Self::File)
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::domain::{
        image::ImageFormat,
        upload::{UploadKind, file_type},
    };

    #[rstest]
    #[case("zz.pdf", "application/pdf")]
    #[case(
        "zz.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    )]
    #[case(
        "zz.pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    )]
    #[case("zz.mp4", "video/mp4")]
    #[case("zz.mp3", "audio/mpeg")]
    #[case("zz.txt", "text/plain")]
    #[case::upper(
        "ZZ.DOCX",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    )]
    #[case::unknown("zz.xyz", "application/octet-stream")]
    #[case::no_extension("zz", "application/octet-stream")]
    #[case::trailing_dot("zz.", "application/octet-stream")]
    fn file_type_follows_the_extension(#[case] name: &str, #[case] expected: &str) {
        assert_eq!(file_type(name), expected);
    }

    #[rstest]
    #[case::image_as_image(b"GIF89a", true, Some(UploadKind::Image(ImageFormat::Gif)))]
    #[case::image_as_file(b"GIF89a", false, Some(UploadKind::File))]
    #[case::other_as_file(b"%PDF-1.7", false, Some(UploadKind::File))]
    #[case::other_as_image(b"%PDF-1.7", true, None)]
    fn kind_comes_from_the_bytes_and_the_asked_kind(
        #[case] bytes: &[u8],
        #[case] as_image: bool,
        #[case] expected: Option<UploadKind>,
    ) {
        assert_eq!(UploadKind::of(bytes, as_image), expected);
    }
}
