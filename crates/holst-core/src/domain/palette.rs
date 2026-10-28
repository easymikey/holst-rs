//! The board palette: color tokens such as `yellow4` to sRGB, light theme.

use std::{cmp::Ordering, fmt, str::FromStr};

use serde::Serialize;

use crate::error::{Error, Result, board_change::BoardChangeError};

#[rustfmt::skip]
const SHADES: [(&str, [&str; 12]); 23] = [
    ("amber", ["#fefdfb", "#fff9ed", "#fff3d0", "#ffecb7", "#ffe0a1", "#f5d08c", "#e4bb78", "#d6a35c", "#ffc53d", "#ffba1a", "#915930", "#4f3422"]),
    ("blue", ["#fbfdff", "#f5faff", "#edf6ff", "#e1f0ff", "#cee7fe", "#b7d9f8", "#96c7f2", "#5eb0ef", "#0090ff", "#0880ea", "#0b68cb", "#113264"]),
    ("brown", ["#fefdfc", "#fcf9f6", "#f8f1ea", "#f4e9dd", "#efddcc", "#e8cdb5", "#ddb896", "#d09e72", "#ad7f58", "#9e7352", "#815e46", "#3e332e"]),
    ("crimson", ["#fffcfd", "#fff7fb", "#feeff6", "#fce5f0", "#f9d8e7", "#f4c6db", "#edadc8", "#e58fb1", "#e93d82", "#dc3175", "#cb1d63", "#621639"]),
    ("cyan", ["#fafdfe", "#f2fcfd", "#e7f9fb", "#d8f3f6", "#c4eaef", "#aadee6", "#84cdda", "#3db9cf", "#05a2c2", "#0894b3", "#0c7792", "#0d3c48"]),
    ("grass", ["#fbfefb", "#f3fcf3", "#ebf9eb", "#dff3df", "#ceebcf", "#b7dfba", "#97cf9c", "#65ba75", "#46a758", "#3d9a50", "#297c3b", "#203c25"]),
    ("gray", ["#fcfcfc", "#f9f9f9", "#f1f1f1", "#ebebeb", "#e4e4e4", "#dddddd", "#d4d4d4", "#b3b3b3", "#8d8d8d", "#808080", "#171717", "#202020"]),
    ("green", ["#fbfefc", "#f2fcf5", "#e9f9ee", "#ddf3e4", "#ccebd7", "#b4dfc4", "#92ceac", "#5bb98c", "#30a46c", "#299764", "#18794e", "#193b2d"]),
    ("indigo", ["#fdfdfe", "#f8faff", "#f0f4ff", "#e6edfe", "#d9e2fc", "#c6d4f9", "#aec0f5", "#8da4ef", "#3e63dd", "#3a5ccc", "#3451b2", "#1f2d5c"]),
    ("jade", ["#fbfefd", "#effdf6", "#e4faef", "#d7f4e6", "#c6ecdb", "#b0e0cc", "#8fcfb9", "#56ba9f", "#29a383", "#259678", "#1a7a5e", "#1d3b31"]),
    ("lime", ["#fcfdfa", "#f7fcf0", "#edfada", "#e2f5c4", "#d5edaf", "#c6de99", "#b2ca7f", "#9ab654", "#bdee63", "#b0e64c", "#59682c", "#37401c"]),
    ("mint", ["#f9fefd", "#effefa", "#ddfbf3", "#ccf7ec", "#bbeee2", "#a6e1d3", "#87d0bf", "#51bda7", "#86ead4", "#7fe1cc", "#27756a", "#16433c"]),
    ("orange", ["#fefcfb", "#fff8f4", "#ffedd5", "#ffe0bb", "#ffd3a4", "#ffc291", "#ffaa7d", "#ed8a5c", "#f76808", "#ed5f00", "#99543a", "#582d1d"]),
    ("pink", ["#fffcfe", "#fff7fc", "#feeef8", "#fce5f3", "#f9d8ec", "#f3c6e2", "#ecadd4", "#e38ec3", "#d6409f", "#cd3093", "#c41c87", "#651249"]),
    ("plum", ["#fefcff", "#fff8ff", "#fceffc", "#f9e5f9", "#f3d9f4", "#ebc8ed", "#dfafe3", "#cf91d8", "#ab4aba", "#a43cb4", "#9c2bad", "#53195d"]),
    ("purple", ["#fefcfe", "#fdfaff", "#f9f1fe", "#f3e7fc", "#eddbf9", "#e3ccf4", "#d3b4ed", "#be93e4", "#8e4ec6", "#8445bc", "#793aaf", "#402060"]),
    ("red", ["#fffcfc", "#fff7f7", "#ffefef", "#ffe5e5", "#fdd8d8", "#f9c6c6", "#f3aeaf", "#eb9091", "#e5484d", "#d93d42", "#c62a2f", "#641723"]),
    ("sky", ["#f9feff", "#f1fcff", "#e2f9ff", "#d2f4fd", "#bfebf8", "#a5dced", "#82cae0", "#46b8d8", "#7ce2fe", "#72dbf8", "#256e93", "#1a404d"]),
    ("teal", ["#fafefd", "#f1fcfa", "#e7f9f5", "#d9f3ee", "#c7ebe5", "#afdfd7", "#8dcec3", "#53b9ab", "#12a594", "#0e9888", "#067a6f", "#0d3d38"]),
    ("tomato", ["#fffcfc", "#fff8f7", "#fff0ee", "#ffe6e2", "#fdd8d3", "#fac7be", "#f3b0a2", "#ea9280", "#e54d2e", "#d84224", "#c33113", "#5c271f"]),
    ("violet", ["#fdfcfe", "#fbfaff", "#f5f2ff", "#ede9fe", "#e4defc", "#d7cff9", "#c4b8f3", "#aa99ec", "#6e56cf", "#644fc1", "#5746af", "#2f265f"]),
    ("white", ["#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff", "#ffffff"]),
    ("yellow", ["#fdfdf9", "#fffbe0", "#fff8c6", "#fcf3af", "#f7ea9b", "#ecdd85", "#dac56e", "#ffe366", "#fbe32d", "#f9da10", "#775f28", "#473b1f"]),
];

pub const BOARD: &str = "#f5f5f5";

pub const TEXT_COLOR: &str = "#202020";

/// The `#rrggbb` color of a palette token, e.g. `yellow4`; `None` for
/// anything else.
///
/// ```
/// use holst_core::domain::palette::hex;
///
/// assert_eq!(hex("yellow4"), Some("#fcf3af"));
/// assert_eq!(hex("yellow13"), None);
/// ```
#[must_use]
pub fn hex(token: &str) -> Option<&'static str> {
    let (shades, shade) = shades_of(token)?;
    shades.get(usize::from(shade).checked_sub(1)?).copied()
}

/// A token's name and shade, e.g. `("yellow", 4)`; `None` for anything else.
#[must_use]
pub fn split(token: &str) -> Option<(&str, u8)> {
    let name = token.trim_end_matches(|c: char| c.is_ascii_digit());
    shades_of(token).map(|(_, shade)| (name, shade))
}

/// The twelve shades of a token's color and the shade it names; a shade has
/// no leading zero and runs from 1 to 12.
fn shades_of(token: &str) -> Option<(&'static [&'static str; 12], u8)> {
    let name = token.trim_end_matches(|c: char| c.is_ascii_digit());
    let shade: u8 = Some(token.get(name.len()..)?)
        .filter(|digits| !digits.starts_with('0'))?
        .parse()
        .ok()
        .filter(|shade| (1..=12).contains(shade))?;
    let at = SHADES
        .binary_search_by(|(known, _)| known.cmp(&name))
        .ok()?;
    SHADES.get(at).map(|(_, shades)| (shades, shade))
}

/// The color of a stored value: a palette token or a 24-bit RGB number.
#[must_use]
pub fn of_value(value: &serde_json::Value) -> Option<String> {
    value.as_str().and_then(hex).map(str::to_owned).or_else(|| {
        value
            .as_u64()
            .map(|rgb| format!("#{:06x}", rgb & 0x00ff_ffff))
    })
}

fn channels(hex: &str) -> [f64; 3] {
    [1, 3, 5].map(|at| {
        hex.get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(255.0, f64::from)
    })
}

/// Text that reads on `fill` by the web client's rule: `gray12` on light
/// colors, white on dark ones.
///
/// ```
/// use holst_core::domain::palette::{TEXT_COLOR, text_color_on};
///
/// assert_eq!(text_color_on("#fcf3af"), TEXT_COLOR);
/// assert_eq!(text_color_on("#202020"), "#ffffff");
/// ```
#[must_use]
pub fn text_color_on(fill: &str) -> &'static str {
    let [r, g, b] = channels(fill);
    let luma = (299.0 * r + 587.0 * g + 114.0 * b) / 1000.0;
    match luma.total_cmp(&160.0) {
        Ordering::Greater => TEXT_COLOR,
        Ordering::Less | Ordering::Equal => "#ffffff",
    }
}

/// `hex` with every channel scaled by `factor`, e.g. 0.8 for 20% darker.
#[must_use]
pub fn scaled(hex: &str, factor: f64) -> String {
    let [r, g, b] = channels(hex).map(|c| two_digits(c * factor));
    format!("#{r}{g}{b}")
}

fn two_digits(channel: f64) -> String {
    let value = (0_u8..=255)
        .find(|v| f64::from(*v) >= channel.round())
        .unwrap_or(255);
    format!("{value:02x}")
}

/// A palette token: a color name and a shade 1–12, e.g. `yellow4`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Color(String);

impl Color {
    /// The token as the document stores it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Color {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let token = raw.trim().to_ascii_lowercase();
        hex(&token).map(|_| Self(token)).ok_or_else(|| {
            Error::BoardChange(BoardChangeError::NotColor { raw: raw.into() })
        })
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prop_assert, prop_assert_eq, proptest};

    use crate::domain::palette::{
        Color,
        SHADES,
        TEXT_COLOR,
        hex,
        of_value,
        scaled,
        split,
        text_color_on,
    };

    proptest! {
        /// Whatever the palette draws, a person can type, and the reverse.
        #[test]
        fn every_palette_colour_parses_and_draws(
            at in 0..SHADES.len(),
            shade in 1_u8..=12,
        ) {
            let token = SHADES.get(at).map(|(name, _)| format!("{name}{shade}")).unwrap_or_default();
            prop_assert!(hex(&token).is_some());
            prop_assert_eq!(token.parse::<Color>().map(|color| color.to_string()).ok(), Some(token));
        }
    }

    #[test]
    fn names_are_sorted_for_the_binary_search() {
        assert!(SHADES.is_sorted_by(|(one, _), (other, _)| one < other));
    }

    #[test]
    fn tokens_map_to_the_web_client_colours() {
        assert_eq!(hex("white3"), Some("#ffffff"));
        assert_eq!(hex("gray12"), Some("#202020"));
        assert_eq!(hex("red9"), Some("#e5484d"));
        assert_eq!(hex("jade1"), Some("#fbfefd"));
        let unknown = [
            "", "yellow", "yellow0", "yellow04", "yellow13", "black4", "4", "yellow-1",
        ];
        assert!(unknown.iter().all(|token| hex(token).is_none()));
        assert!("Gray12".parse::<Color>().is_ok());
    }

    #[test]
    fn values_tokens_and_shading() {
        assert_eq!(split("yellow4"), Some(("yellow", 4)));
        assert_eq!(split("yellow13"), None);
        assert_eq!(
            of_value(&serde_json::json!("red9")).as_deref(),
            Some("#e5484d")
        );
        assert_eq!(
            of_value(&serde_json::json!(0x00ab_cdef)).as_deref(),
            Some("#abcdef")
        );
        assert_eq!(of_value(&serde_json::Value::Null), None);
        assert_eq!(scaled("#ff8000", 0.5), "#804000");
        assert_eq!(text_color_on("not a colour"), TEXT_COLOR);
    }
}
