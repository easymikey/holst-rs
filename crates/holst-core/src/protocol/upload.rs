//! File upload: the `multipart/form-data` body.

use std::{fmt, fmt::Write as _};

#[must_use]
pub(crate) fn multipart_body(
    boundary: &str,
    field: &str,
    file_name: &str,
    content_type: &str,
    bytes: &[u8],
) -> Vec<u8> {
    let file_name = fmt::from_fn(|f| {
        file_name.chars().try_for_each(|ch| match ch {
            '"' => f.write_str("%22"),
            '\r' => f.write_str("%0D"),
            '\n' => f.write_str("%0A"),
            ch => f.write_char(ch),
        })
    });
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; \
         filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
    );
    let tail = format!("\r\n--{boundary}--\r\n");
    [head.as_bytes(), bytes, tail.as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use crate::protocol::upload::multipart_body;

    #[test]
    fn multipart_body_wraps_bytes_and_escapes_file_name() {
        let body =
            multipart_body("BND", "file", "a\".png\r\nX: y", "image/png", b"\x89PNG");
        let expected: &[u8] =
            b"--BND\r\nContent-Disposition: form-data; name=\"file\"; \
            filename=\"a%22.png%0D%0AX: y\"\r\nContent-Type: image/png\
            \r\n\r\n\x89PNG\r\n--BND--\r\n";
        assert_eq!(body, expected);
    }

    #[test]
    fn non_ascii_file_name_is_kept() {
        let body = multipart_body("B", "file", "план.png", "image/png", b"");
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("filename=\"план.png\""), "{text}");
    }
}
