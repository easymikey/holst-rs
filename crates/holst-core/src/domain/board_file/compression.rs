//! Gzip around a saved board: packed on save, either form read back; pure.

use std::{
    borrow::Cow,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use flate2::{Compression as GzipLevel, bufread::MultiGzDecoder, write::GzEncoder};

use crate::error::{Error, Result, local_data::LocalDataError};

pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub const GZIP_SUFFIX: &str = ".gz";

pub const MAX_UNPACKED: u64 = 1 << 30;

/// How a saved state is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    Plain,
}

/// Whether the bytes are a gzip stream.
#[must_use]
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(&GZIP_MAGIC)
}

/// The bytes as `compression` writes them; gzip bytes are never packed twice.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::Pack`] if the encoder fails (it
/// writes to memory, so it should not).
pub fn compressed(bytes: &[u8], compression: Compression) -> Result<Cow<'_, [u8]>> {
    match compression {
        Compression::Gzip if !is_compressed(bytes) => gzip(bytes).map(Cow::Owned),
        Compression::Gzip | Compression::Plain => Ok(Cow::Borrowed(bytes)),
    }
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    // imperative: the encoder writes through `&mut self`
    let mut encoder =
        GzEncoder::new(Vec::with_capacity(bytes.len() / 3), GzipLevel::default());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .map_err(|err| Error::LocalData(LocalDataError::Pack(err)))
}

/// The state inside: gzip unpacked (checksum verified), anything else as is.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::Unpack`] for a broken stream,
/// [`crate::error::local_data::LocalDataError::UnpackTooLarge`] past the limit.
pub fn uncompressed(bytes: &[u8]) -> Result<Cow<'_, [u8]>> {
    if is_compressed(bytes) {
        gunzip(bytes, MAX_UNPACKED).map(Cow::Owned)
    } else {
        Ok(Cow::Borrowed(bytes))
    }
}

fn gunzip(bytes: &[u8], limit: u64) -> Result<Vec<u8>> {
    // Boards pack about 3.3 to 1: one allocation, not a dozen regrowths.
    let guess = u64::try_from(bytes.len().saturating_mul(4))
        .unwrap_or(limit)
        .min(limit);
    // imperative: `read_to_end` fills a caller-owned buffer
    let mut out = Vec::with_capacity(usize::try_from(guess).unwrap_or_default());
    MultiGzDecoder::new(bytes)
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(LocalDataError::Unpack)?;
    let fits = u64::try_from(out.len()).is_ok_and(|len| len <= limit);
    fits.then_some(out)
        .ok_or(Error::LocalData(LocalDataError::UnpackTooLarge {
            limit_mib: limit >> 20,
        }))
}

/// Where a file goes: gzip gains `.gz` when the name lacks it.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::NoFileName`];
/// [`crate::error::local_data::LocalDataError::PlainGz`] for plain bytes under
/// a `.gz` name.
pub fn file_name(path: &Path, compression: Compression) -> Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        Error::LocalData(LocalDataError::NoFileName {
            path: path.to_path_buf(),
        })
    })?;
    let gz = name
        .to_str()
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(GZIP_SUFFIX));
    match (compression, gz) {
        (Compression::Gzip, true) | (Compression::Plain, false) => {
            Ok(path.to_path_buf())
        }
        (Compression::Gzip, false) => {
            let packed: std::ffi::OsString = [name, std::ffi::OsStr::new(GZIP_SUFFIX)]
                .into_iter()
                .collect();
            Ok(path.with_file_name(packed))
        }
        (Compression::Plain, true) => Err(Error::LocalData(LocalDataError::PlainGz {
            path: path.to_path_buf(),
        })),
    }
}

/// The name without a trailing `.gz`, for files written beside it.
#[must_use]
pub fn stem(path: &Path) -> PathBuf {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| {
            let cut = name.len().checked_sub(GZIP_SUFFIX.len())?;
            let (head, tail) = (name.get(..cut)?, name.get(cut..)?);
            (cut > 0 && tail.eq_ignore_ascii_case(GZIP_SUFFIX))
                .then(|| path.with_file_name(head))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::{
        borrow::Cow,
        path::{Path, PathBuf},
    };

    use rstest::rstest;

    use crate::{
        domain::board_file::compression::{
            Compression,
            GZIP_MAGIC,
            compressed,
            file_name,
            gunzip,
            is_compressed,
            stem,
            uncompressed,
        },
        error::{Error, Result, local_data::LocalDataError},
    };

    #[test]
    fn gzip_round_trips_and_plain_passes_through() -> Result<()> {
        let state = b"\x00\x01yjs update bytes".repeat(100);
        let gz = compressed(&state, Compression::Gzip)?;
        assert!(is_compressed(&gz) && gz.len() < state.len());
        assert_eq!(uncompressed(&gz)?, &state[..]);
        assert!(matches!(
            compressed(&state, Compression::Plain)?,
            Cow::Borrowed(_)
        ));
        assert!(matches!(uncompressed(&state)?, Cow::Borrowed(_)));
        assert!(
            matches!(compressed(&gz, Compression::Gzip)?, Cow::Borrowed(_)),
            "packed twice"
        );
        Ok(())
    }

    #[test]
    fn packing_is_deterministic() -> Result<()> {
        let state = b"board".repeat(50);
        assert_eq!(
            compressed(&state, Compression::Gzip)?,
            compressed(&state, Compression::Gzip)?
        );
        Ok(())
    }

    #[test]
    fn a_broken_or_cut_stream_is_an_error() -> Result<()> {
        let gz =
            compressed(&b"board state".repeat(40), Compression::Gzip)?.into_owned();
        let cut = gz.get(..gz.len() / 2).unwrap_or_default();
        assert!(matches!(
            uncompressed(cut),
            Err(Error::LocalData(LocalDataError::Unpack(_)))
        ));
        let target = gz.len().saturating_sub(6);
        let flipped: Vec<u8> = gz
            .iter()
            .enumerate()
            .map(|(at, byte)| byte ^ (u8::from(at == target) * 0xff))
            .collect();
        assert!(
            uncompressed(&flipped).is_err(),
            "the checksum was not verified"
        );
        assert!(matches!(
            uncompressed(&GZIP_MAGIC),
            Err(Error::LocalData(LocalDataError::Unpack(_)))
        ));
        Ok(())
    }

    #[test]
    fn a_stream_past_the_limit_is_refused() -> Result<()> {
        let gz = compressed(&[0; 4096], Compression::Gzip)?.into_owned();
        assert!(matches!(
            gunzip(&gz, 4095),
            Err(Error::LocalData(LocalDataError::UnpackTooLarge { .. }))
        ));
        assert_eq!(gunzip(&gz, 4096)?.len(), 4096);
        Ok(())
    }

    #[test]
    fn every_member_is_read() -> Result<()> {
        let both = [
            compressed(b"first ", Compression::Gzip)?.into_owned(),
            compressed(b"second", Compression::Gzip)?.into_owned(),
        ]
        .concat();
        assert_eq!(uncompressed(&both)?, &b"first second"[..]);
        Ok(())
    }

    #[rstest]
    #[case::gains_gz(
        "dir/general.ydoc",
        Compression::Gzip,
        Some("dir/general.ydoc.gz")
    )]
    #[case::keeps_gz("general.ydoc.gz", Compression::Gzip, Some("general.ydoc.gz"))]
    #[case::any_case("general.YDOC.GZ", Compression::Gzip, Some("general.YDOC.GZ"))]
    #[case::plain("general.ydoc", Compression::Plain, Some("general.ydoc"))]
    #[case::plain_gz("general.ydoc.gz", Compression::Plain, None)]
    #[case::no_name("..", Compression::Gzip, None)]
    fn the_name_says_how_the_file_is_packed(
        #[case] path: &str,
        #[case] compression: Compression,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            file_name(Path::new(path), compression).ok(),
            expected.map(PathBuf::from)
        );
    }

    #[rstest]
    #[case("dir/general.ydoc.gz", "dir/general.ydoc")]
    #[case("general.ydoc.GZ", "general.ydoc")]
    #[case("general.ydoc", "general.ydoc")]
    #[case(".gz", ".gz")]
    fn the_stem_drops_only_a_trailing_gz(#[case] path: &str, #[case] expected: &str) {
        assert_eq!(stem(Path::new(path)), PathBuf::from(expected));
    }
}
