//! lib0 variable-length encoding: integers, bytes and strings, no I/O.

use crate::error::{Error, Result, board_socket::BoardSocketError};

#[must_use]
pub(crate) fn var_uint(value: u64) -> Vec<u8> {
    let groups: Vec<u8> = std::iter::successors(Some(value), |rest| {
        Some(rest >> 7).filter(|next| *next > 0)
    })
    .map(|rest| u8::try_from(rest & 0x7f).unwrap_or_default())
    .collect();
    let last = groups.len().saturating_sub(1);
    groups
        .iter()
        .enumerate()
        .map(|(at, group)| group | (u8::from(at < last) << 7))
        .collect()
}

#[must_use]
pub(crate) fn var_string(text: &str) -> Vec<u8> {
    var_bytes(text.as_bytes())
}

#[must_use]
pub(crate) fn var_bytes(bytes: &[u8]) -> Vec<u8> {
    [
        var_uint(u64::try_from(bytes.len()).unwrap_or(u64::MAX)).as_slice(),
        bytes,
    ]
    .concat()
}

/// An unsigned varint and the bytes after it.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::MalformedFrame`] for a cut
/// or over-long varint.
pub(crate) fn read_var_uint(input: &[u8]) -> Result<(u64, &[u8])> {
    let end = input
        .iter()
        .position(|byte| byte & 0x80 == 0)
        .filter(|end| *end < 10)
        .ok_or(Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "a cut varint",
        }))?;
    let (head, rest) = input.split_at(end + 1);
    let wide = head
        .iter()
        .zip((0_u32..).step_by(7))
        .fold(0_u128, |value, (byte, shift)| {
            value | (u128::from(byte & 0x7f) << shift)
        });
    u64::try_from(wide).map(|value| (value, rest)).map_err(|_| {
        Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "a varint over 64 bits",
        })
    })
}

/// A lib0 signed varint: 6 value bits and a sign bit in the first byte.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::MalformedFrame`] for a cut
/// or over-long varint.
pub(crate) fn read_var_int(input: &[u8]) -> Result<(i64, &[u8])> {
    let (first, after) = input.split_first().ok_or(Error::BoardSocket(
        BoardSocketError::MalformedFrame {
            what: "a cut signed varint",
        },
    ))?;
    let low = i64::from(first & 0x3f);
    let (high, rest) = match first & 0x80 {
        0 => (0, after),
        _ => read_var_uint(after).and_then(|(high, rest)| {
            i64::try_from(high)
                .ok()
                .and_then(|high| high.checked_mul(64))
                .map(|high| (high, rest))
                .ok_or(Error::BoardSocket(BoardSocketError::MalformedFrame {
                    what: "a signed varint over 63 bits",
                }))
        })?,
    };
    let magnitude = high | low;
    let signed = match first & 0x40 {
        0 => magnitude,
        _ => -magnitude,
    };
    Ok((signed, rest))
}

/// lib0 bytes and the input after them.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::MalformedFrame`] if the
/// length runs past the input.
pub(crate) fn read_var_bytes(input: &[u8]) -> Result<(&[u8], &[u8])> {
    let (len, rest) = read_var_uint(input)?;
    usize::try_from(len)
        .ok()
        .filter(|len| *len <= rest.len())
        .map(|len| rest.split_at(len))
        .ok_or(Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "bytes longer than the frame",
        }))
}

/// A lib0 string and the input after it.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::MalformedFrame`] for a cut
/// string or invalid UTF-8.
pub(crate) fn read_var_string(input: &[u8]) -> Result<(&str, &[u8])> {
    let (bytes, rest) = read_var_bytes(input)?;
    std::str::from_utf8(bytes)
        .map(|text| (text, rest))
        .map_err(|_| {
            Error::BoardSocket(BoardSocketError::MalformedFrame {
                what: "a string that is not UTF-8",
            })
        })
}
