//! The cookie jar as a value: stored line, `Set-Cookie`, `Cookie` header.

use std::{collections::BTreeMap, convert::Infallible, fmt, str::FromStr};

use secrecy::{ExposeSecret, SecretString};

use crate::domain::time::Timestamp;

/// Cookies by name; values are secrets, `Debug` shows names only.
#[derive(Clone, Default)]
pub struct CookieJar(BTreeMap<String, SecretString>);

impl CookieJar {
    /// How many cookies the jar holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// No cookies at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Cookie values, still secret, in name order.
    pub fn values(&self) -> impl Iterator<Item = &SecretString> {
        self.0.values()
    }

    /// Cookie names, sorted. Safe to log: names are not secret.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// The jar after a response: set cookies replace old values, cleared ones
    /// go.
    #[must_use]
    pub fn merge(self, fresh: impl IntoIterator<Item = CookieChange>) -> Self {
        Self(fresh.into_iter().fold(self.0, |mut jar, change| {
            // The replaced value is dropped here, and zeroed as a secret.
            match change {
                CookieChange::Cleared { name } => drop(jar.remove(&name)),
                CookieChange::Set { name, value } => {
                    drop(jar.insert(name, SecretString::from(value)));
                }
            }
            jar
        }))
    }

    /// The `Cookie` header value `a=1; b=2`, still secret.
    #[must_use]
    pub fn header(&self) -> SecretString {
        let pairs = self.0.iter().map(|(name, value)| {
            fmt::from_fn(move |f| write!(f, "{name}={}", value.expose_secret()))
        });
        SecretString::from(crate::domain::text::joined(pairs, "; ").to_string())
    }
}

impl fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.names()).finish()
    }
}

impl PartialEq for CookieJar {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(|((a, x), (b, y))| {
                a == b && x.expose_secret() == y.expose_secret()
            })
    }
}

impl Eq for CookieJar {}

impl FromIterator<(String, String)> for CookieJar {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(pairs: I) -> Self {
        Self(
            pairs
                .into_iter()
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| (name, SecretString::from(value)))
                .collect(),
        )
    }
}

impl FromStr for CookieJar {
    type Err = Infallible;

    fn from_str(stored: &str) -> Result<Self, Infallible> {
        Ok(stored
            .split(';')
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .collect())
    }
}

/// What a `Set-Cookie` does to the jar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookieChange {
    /// The cookie takes this value.
    Set { name: String, value: String },
    /// The cookie ended: empty value, `Max-Age` <= 0 or `Expires` in the past.
    Cleared { name: String },
}

impl CookieChange {
    /// The cookie this change is about.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Set { name, .. } | Self::Cleared { name } => name,
        }
    }
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Returns the Unix seconds of an HTTP date such as `Thu, 01 Jan 1970 00:00:00
/// GMT` (also `01-Jan-70` and a missing weekday); the zone is taken as GMT.
fn http_date_seconds(text: &str) -> Option<i64> {
    let tokens: Vec<&str> = text
        .split([' ', ',', '-', ':'])
        .filter(|token| !token.is_empty())
        .collect();
    let fields = match tokens.as_slice() {
        [weekday, rest @ ..] if weekday.starts_with(char::is_alphabetic) => rest,
        all => all,
    };
    match fields {
        [day, month, year, hour, minute, second, ..] => {
            let month = MONTHS
                .iter()
                .position(|name| {
                    month
                        .get(..3)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
                })
                .and_then(|index| i64::try_from(index + 1).ok())?;
            // Past 9999 the date is no date and cannot overflow the day count.
            let year_number = year
                .parse::<i64>()
                .ok()
                .filter(|year| (0..=9999).contains(year))?;
            // RFC 6265: two digits are 1970..=2069.
            let year_number = match year.len() {
                2 => year_number + 1900 + 100 * i64::from(year_number < 70),
                _ => year_number,
            };
            let day = day
                .parse::<i64>()
                .ok()
                .filter(|day| (1..=31).contains(day))?;
            let clock = |field: &str, limit: i64| {
                field
                    .parse::<i64>()
                    .ok()
                    .filter(|value| (0..limit).contains(value))
            };
            let time =
                clock(hour, 24)? * 3600 + clock(minute, 60)? * 60 + clock(second, 61)?;
            // Days from the civil date, as in Howard Hinnant's `days_from_civil`.
            let shifted_year = year_number - i64::from(month <= 2);
            let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
            let year_of_era = shifted_year.rem_euclid(400);
            let day_of_era =
                year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
            let days = shifted_year.div_euclid(400) * 146_097 + day_of_era - 719_468;
            Some(days * 86_400 + time)
        }
        _ => None,
    }
}

/// Reads a `Set-Cookie` as a change of the jar at `now`; only `Max-Age`
/// and `Expires` matter among the attributes, the rest do not matter to one
/// client. `Max-Age` wins over `Expires`.
///
/// ```
/// use holst_core::domain::{
///     cookie::{CookieChange, parse_set_cookie},
///     time::Timestamp,
/// };
///
/// let now = Timestamp::from_millis(0);
/// assert_eq!(
///     parse_set_cookie("sid=abc; Path=/; HttpOnly", now),
///     Some(CookieChange::Set {
///         name: "sid".into(),
///         value: "abc".into()
///     })
/// );
/// assert_eq!(parse_set_cookie("=nameless", now), None);
/// ```
#[must_use]
pub fn parse_set_cookie(raw: &str, received_at: Timestamp) -> Option<CookieChange> {
    let (pair, attributes) = raw.split_once(';').unwrap_or((raw, ""));
    let (name, value) = pair.trim().split_once('=')?;
    let attribute = |wanted: &str| {
        attributes
            .split(';')
            .filter_map(|attribute| attribute.split_once('='))
            .filter(|(key, _)| key.trim().eq_ignore_ascii_case(wanted))
            .map(|(_, text)| text.trim())
            .next_back()
    };
    let received_at = i64::try_from(received_at.as_millis() / 1000).unwrap_or(i64::MAX);
    let ended = attribute("max-age")
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .map_or_else(
            || {
                attribute("expires")
                    .and_then(http_date_seconds)
                    .is_some_and(|expires| expires <= received_at)
            },
            |seconds| seconds <= 0,
        );
    let kept = Some(value.trim()).filter(|value| !ended && !value.is_empty());
    Some(name.trim())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .map(|name| match kept {
            Some(text) => CookieChange::Set {
                name,
                value: text.to_string(),
            },
            None => CookieChange::Cleared { name },
        })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use secrecy::ExposeSecret;

    use crate::domain::{
        cookie::{CookieChange, CookieJar, parse_set_cookie},
        time::Timestamp,
    };

    // 2023-11-14 22:13:20 GMT.
    const NOW: Timestamp = Timestamp::from_millis(1_700_000_000_000);

    fn jar(stored: &str) -> CookieJar {
        let Ok(jar) = stored.parse::<CookieJar>();
        jar
    }

    fn set(name: &str, value: &str) -> CookieChange {
        CookieChange::Set {
            name: name.into(),
            value: value.into(),
        }
    }

    fn cleared(name: &str) -> CookieChange {
        CookieChange::Cleared { name: name.into() }
    }

    #[rstest]
    #[case::plain("sessionToken=abc", Some(set("sessionToken", "abc")))]
    #[case::with_attributes("a=b; Path=/; HttpOnly; Secure", Some(set("a", "b")))]
    #[case::empty_value("a=; Path=/", Some(cleared("a")))]
    #[case::max_age_zero("sid=deleted; Max-Age=0", Some(cleared("sid")))]
    #[case::max_age_negative("sid=x; path=/; max-age=-1", Some(cleared("sid")))]
    #[case::max_age_positive("sid=x; Max-Age=60", Some(set("sid", "x")))]
    #[case::expired(
        "sid=x; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        Some(cleared("sid"))
    )]
    #[case::expired_yesterday(
        "sid=x; expires=Mon, 13 Nov 2023 22:13:20 GMT",
        Some(cleared("sid"))
    )]
    #[case::expired_with_dashes(
        "sid=x; Expires=Thu, 01-Jan-1970 00:00:00 GMT",
        Some(cleared("sid"))
    )]
    #[case::expired_two_digit_year(
        "sid=x; Expires=Thursday, 01-Jan-70 00:00:00 GMT",
        Some(cleared("sid"))
    )]
    #[case::expires_in_the_future(
        "sid=x; Expires=Fri, 01 Jan 2100 00:00:00 GMT",
        Some(set("sid", "x"))
    )]
    #[case::two_digit_year_in_the_future(
        "sid=x; Expires=Fri, 01-Jan-69 00:00:00 GMT",
        Some(set("sid", "x"))
    )]
    #[case::max_age_wins(
        "sid=x; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=60",
        Some(set("sid", "x"))
    )]
    #[case::unreadable_date("sid=x; Expires=soon", Some(set("sid", "x")))]
    #[case::year_past_any_calendar(
        "sid=x; Expires=Thu, 01 Jan 99999999999999 00:00:00 GMT",
        Some(set("sid", "x"))
    )]
    #[case::spaces(" a = b ; Path=/", Some(set("a", "b")))]
    #[case::no_value("broken", None)]
    #[case::no_name("=value", None)]
    fn set_cookie_is_read_as_a_change(
        #[case] raw: &str,
        #[case] expected: Option<CookieChange>,
    ) {
        assert_eq!(parse_set_cookie(raw, NOW), expected);
    }

    #[test]
    fn stored_cookies_come_back_sorted() {
        let jar = jar("b=2; a=1; =x; junk");
        assert_eq!(jar.header().expose_secret(), "a=1; b=2");
        assert_eq!(jar.len(), 2);
    }

    #[test]
    fn fresh_cookies_replace_and_cleared_ones_go() {
        let jar = jar("a=1; b=2").merge([set("b", "3"), cleared("a"), set("c", "4")]);
        assert_eq!(jar.header().expose_secret(), "b=3; c=4");
    }

    #[test]
    fn debug_shows_names_but_never_values() {
        let shown = format!("{:?}", jar("session=s3cr3t; theme=dark"));
        assert_eq!(shown, r#"{"session", "theme"}"#);
    }
}
