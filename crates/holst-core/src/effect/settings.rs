//! The settings of one run, built once in `main` and passed down.

use std::{ffi::OsString, path::PathBuf};

use crate::{
    domain::url::BaseUrl,
    error::{Error, Result, input::InputError},
};

pub const BASE_URL_VAR: &str = "HOLST_BASE_URL";
pub const HOME_VAR: &str = "HOLST_HOME";
pub const LOG_VAR: &str = "HOLST_LOG";
pub const CACHE_VAR: &str = "HOLST_CACHE";
pub const CACHE_DIR_VAR: &str = "HOLST_CACHE_DIR";
pub const CACHE_MAX_VAR: &str = "HOLST_CACHE_MAX";
pub const CACHE_TRUST_VAR: &str = "HOLST_CACHE_TRUST";
pub const DEFAULT_CACHE_MAX: u64 = 256 << 20;
pub const DEFAULT_CACHE_TRUST_MILLIS: u64 = 24 * 60 * 60 * 1000;
const HOME_CACHE_DIR: &str = "cache";

/// What a run works with: the API address, the login location and the logs.
#[derive(Debug, Clone)]
pub struct Settings {
    pub base_url: BaseUrl,
    pub location: SessionLocation,
    pub log: LogSettings,
    pub cache: CacheSettings,
}

/// Where the board cache lives and how large it may grow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSettings {
    pub root: Option<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub host: String,
    pub max_bytes: u64,
    pub trust_millis: u64,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            root: None,
            roots: Vec::new(),
            host: String::new(),
            max_bytes: DEFAULT_CACHE_MAX,
            trust_millis: DEFAULT_CACHE_TRUST_MILLIS,
        }
    }
}

/// How logs on stderr are filtered and colored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSettings {
    pub filter: Option<String>,
    pub color: ColorPreference,
    pub terminal: TerminalType,
}

/// Whether `NO_COLOR` asks for plain lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorPreference {
    #[default]
    Allowed,
    Declined,
}

/// Whether `TERM` names a terminal that cannot show escapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalType {
    #[default]
    Capable,
    Dumb,
}

impl Settings {
    /// The settings from variables that `read_variable` finds by name.
    ///
    /// ```
    /// use holst_core::effect::settings::{SessionLocation, Settings};
    ///
    /// let settings = Settings::from_variables(|name| {
    ///     (name == "HOLST_HOME").then(|| "/tmp/holst".into())
    /// })?;
    /// assert_eq!(settings.base_url.as_str(), "https://app.holst.so");
    /// assert_eq!(settings.location, SessionLocation::Dir("/tmp/holst".into()));
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`crate::error::input::InputError::BaseNotUrl`] or another base address
    /// error.
    pub fn from_variables(
        read_variable: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self> {
        Self::checked(read_variable).map(|(settings, _)| settings)
    }

    /// The settings and a refused cache variable; then the cache is off.
    ///
    /// # Errors
    /// As [`Settings::from_variables`]; a bad cache variable is no error.
    pub fn checked(
        read_variable: impl Fn(&str) -> Option<OsString>,
    ) -> Result<(Self, Option<Error>)> {
        let base_url =
            read_variable(BASE_URL_VAR).and_then(|raw| raw.into_string().ok());
        let base_url = BaseUrl::or_default(base_url.as_deref())?;
        let location = SessionLocation::new(
            read_variable(HOME_VAR).map(PathBuf::from),
            user_home(&read_variable),
        );
        let (cache, refused) =
            CacheSettings::from_variables(&read_variable, &location, &base_url);
        let settings = Self {
            cache,
            base_url,
            location,
            log: LogSettings::from_variables(read_variable),
        };
        Ok((settings, refused))
    }
}

impl CacheSettings {
    /// Cache settings beside the login at `location`, and a refused one.
    ///
    /// ```
    /// use holst_core::{
    ///     domain::url::BaseUrl,
    ///     effect::settings::{CacheSettings, SessionLocation},
    /// };
    ///
    /// let home = SessionLocation::Dir("/tmp/holst".into());
    /// let (cache, refused) =
    ///     CacheSettings::from_variables(&|_| None, &home, &BaseUrl::or_default(None)?);
    /// assert!(refused.is_none());
    /// assert_eq!(cache.root, Some("/tmp/holst/cache".into()));
    /// assert_eq!(cache.host, "app.holst.so");
    /// assert_eq!(cache.max_bytes, 256 << 20);
    /// assert_eq!(cache.trust_millis, 24 * 60 * 60 * 1000);
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    pub fn from_variables(
        read_variable: &impl Fn(&str) -> Option<OsString>,
        location: &SessionLocation,
        base: &BaseUrl,
    ) -> (Self, Option<Error>) {
        let off = read_variable(CACHE_VAR).is_some_and(|value| is_off(&value));
        let named = non_empty(read_variable(CACHE_DIR_VAR)).map(absolute_dir);
        let beside = match location {
            SessionLocation::Dir(dir) => Some(dir.join(HOME_CACHE_DIR)),
            SessionLocation::System { .. } => system_cache(read_variable),
        };
        let max_bytes = read_variable(CACHE_MAX_VAR)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let text = value.to_string_lossy();
                parse_size(&text).ok_or_else(|| {
                    Error::Input(InputError::CacheMaxNotSize {
                        value: text.into_owned(),
                    })
                })
            });
        let trust_ms = read_variable(CACHE_TRUST_VAR)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let text = value.to_string_lossy();
                parse_millis(&text).ok_or_else(|| {
                    Error::Input(InputError::CacheTrustNotDuration {
                        value: text.into_owned(),
                    })
                })
            });
        let (named, named_refused) = split(named);
        let (max_bytes, max_refused) = split(max_bytes);
        let (trust_ms, trust_refused) = split(trust_ms);
        let refused = named_refused.or(max_refused).or(trust_refused);
        let roots =
            beside
                .iter()
                .chain(named.iter())
                .fold(Vec::new(), |mut roots, root| {
                    roots.extend((!roots.contains(root)).then(|| root.clone()));
                    roots
                });
        let settings = Self {
            root: (!off && refused.is_none())
                .then(|| named.or(beside))
                .flatten(),
            roots,
            host: base.host_key(),
            max_bytes: max_bytes.unwrap_or(DEFAULT_CACHE_MAX),
            trust_millis: trust_ms.unwrap_or(DEFAULT_CACHE_TRUST_MILLIS),
        };
        (settings, refused)
    }

    /// The same settings with the cache off, as `--no-cache` asks.
    #[must_use]
    pub fn off(self) -> Self {
        Self { root: None, ..self }
    }
}

fn absolute_dir(dir: PathBuf) -> Result<PathBuf> {
    let value = dir.display().to_string();
    dir.is_absolute()
        .then_some(dir)
        .ok_or_else(|| Error::Input(InputError::CacheDirRelative { value }))
}

fn split<T>(value: Option<Result<T>>) -> (Option<T>, Option<Error>) {
    match value {
        None => (None, None),
        Some(Ok(value)) => (Some(value), None),
        Some(Err(err)) => (None, Some(err)),
    }
}

fn is_off(value: &OsString) -> bool {
    let text = value.to_string_lossy().trim().to_ascii_lowercase();
    ["off", "0", "false", "no"].contains(&text.as_str())
}

#[cfg(target_os = "macos")]
fn system_cache(read_variable: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    non_empty(read_variable("HOME")).map(|home| home.join("Library/Caches/holst"))
}

#[cfg(windows)]
fn system_cache(read_variable: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    non_empty(read_variable("LOCALAPPDATA"))
        .map(|local| local.join("holst").join("cache"))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system_cache(read_variable: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    non_empty(read_variable("XDG_CACHE_HOME"))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("holst"))
        .or_else(|| {
            non_empty(read_variable("HOME")).map(|home| home.join(".cache/holst"))
        })
}

fn non_empty(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// Bytes from `123`, `64K`, `256M`, `1G` (also MB, MiB); powers of 1024.
///
/// ```
/// use holst_core::effect::settings::parse_size;
///
/// assert_eq!(parse_size("256M"), Some(256 << 20));
/// assert_eq!(parse_size("1 GiB"), Some(1 << 30));
/// assert_eq!(parse_size("12"), Some(12));
/// assert_eq!(parse_size("big"), None);
/// ```
#[must_use]
pub fn parse_size(text: &str) -> Option<u64> {
    scaled(text, |unit| match unit {
        "" | "b" => Some(1),
        "k" | "kb" | "kib" => Some(1 << 10),
        "m" | "mb" | "mib" => Some(1 << 20),
        "g" | "gb" | "gib" => Some(1 << 30),
        _ => None,
    })
}

/// The digits of `text` times the factor `factor` gives for the unit word
/// after them, lowercased; `None` for an unknown unit or an overflow.
fn scaled(text: &str, factor: impl Fn(&str) -> Option<u64>) -> Option<u64> {
    let text = text.trim();
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let (number, unit) = text.split_at_checked(digits)?;
    let factor = factor(&unit.trim().to_ascii_lowercase())?;
    number.parse::<u64>().ok()?.checked_mul(factor)
}

/// Milliseconds from `90s`, `30m`, `24h`, `7d` or bare seconds like `3600`.
///
/// ```
/// use holst_core::effect::settings::parse_millis;
///
/// assert_eq!(parse_millis("24h"), Some(86_400_000));
/// assert_eq!(parse_millis("90 s"), Some(90_000));
/// assert_eq!(parse_millis("0"), Some(0));
/// assert_eq!(parse_millis("soon"), None);
/// ```
#[must_use]
pub fn parse_millis(text: &str) -> Option<u64> {
    scaled(text, |unit| match unit {
        "" | "s" | "sec" => Some(1000),
        "m" | "min" => Some(60 * 1000),
        "h" | "hour" | "hours" => Some(60 * 60 * 1000),
        "d" | "day" | "days" => Some(24 * 60 * 60 * 1000),
        _ => None,
    })
}

impl LogSettings {
    /// The log settings from variables that `read_variable` finds by name.
    #[must_use]
    pub fn from_variables(read_variable: impl Fn(&str) -> Option<OsString>) -> Self {
        Self {
            filter: read_variable(LOG_VAR).and_then(|raw| raw.into_string().ok()),
            color: match read_variable("NO_COLOR") {
                Some(value) if !value.is_empty() => ColorPreference::Declined,
                Some(_) | None => ColorPreference::Allowed,
            },
            terminal: match read_variable("TERM") {
                Some(term) if term == "dumb" => TerminalType::Dumb,
                Some(_) | None => TerminalType::Capable,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use std::{collections::HashMap, ffi::OsString, path::PathBuf};

    use rstest::rstest;

    /// An absolute cache directory on every system.
    #[cfg(windows)]
    const C: &str = r"C:\c";
    #[cfg(not(windows))]
    const C: &str = "/c";

    use crate::{
        effect::settings::{
            BASE_URL_VAR,
            CACHE_DIR_VAR,
            CACHE_MAX_VAR,
            CACHE_TRUST_VAR,
            CACHE_VAR,
            ColorPreference,
            DEFAULT_CACHE_MAX,
            DEFAULT_CACHE_TRUST_MILLIS,
            HOME_VAR,
            LOG_VAR,
            LogSettings,
            SessionLocation,
            Settings,
            TerminalType,
        },
        error::{Error, Result, input::InputError},
    };

    fn variables(
        pairs: &[(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |name| map.get(name).map(OsString::from)
    }

    #[test]
    fn nothing_set_means_the_defaults() -> Result<()> {
        let settings = Settings::from_variables(variables(&[]))?;
        assert_eq!(settings.base_url.as_str(), crate::domain::url::DEFAULT_BASE);
        assert_eq!(settings.location, SessionLocation::System { home: None });
        assert_eq!(settings.log, LogSettings::default());
        Ok(())
    }

    #[test]
    fn every_variable_is_read() -> Result<()> {
        let settings = Settings::from_variables(variables(&[
            (BASE_URL_VAR, "http://127.0.0.1:9"),
            (HOME_VAR, "/tmp/h"),
            (LOG_VAR, "debug"),
            ("NO_COLOR", "1"),
            ("TERM", "dumb"),
        ]))?;
        assert_eq!(settings.base_url.as_str(), "http://127.0.0.1:9");
        assert_eq!(settings.location, SessionLocation::Dir("/tmp/h".into()));
        assert_eq!(
            settings.log,
            LogSettings {
                filter: Some("debug".into()),
                color: ColorPreference::Declined,
                terminal: TerminalType::Dumb,
            }
        );
        Ok(())
    }

    #[rstest]
    #[case::empty_no_color("NO_COLOR", "")]
    #[case::empty_home(HOME_VAR, "")]
    #[case::other_term("TERM", "xterm-256color")]
    fn empty_values_are_unset(
        #[case] name: &'static str,
        #[case] value: &'static str,
    ) -> Result<()> {
        let settings = Settings::from_variables(variables(&[(name, value)]))?;
        assert_eq!(settings.location, SessionLocation::System { home: None });
        assert_eq!(settings.log, LogSettings::default());
        Ok(())
    }

    #[rstest]
    #[case::beside_home(&[(HOME_VAR, "/h")], Some("/h/cache"))]
    #[case::named(&[(HOME_VAR, "/h"), (CACHE_DIR_VAR, C)], Some(C))]
    #[case::off(&[(HOME_VAR, "/h"), (CACHE_VAR, "off")], None)]
    #[case::off_in_capitals(&[(CACHE_DIR_VAR, C), (CACHE_VAR, "OFF")], None)]
    #[case::on(&[(CACHE_DIR_VAR, C), (CACHE_VAR, "on")], Some(C))]
    fn the_cache_directory_is_chosen(
        #[case] pairs: &[(&'static str, &'static str)],
        #[case] expected: Option<&str>,
    ) -> Result<()> {
        let settings = Settings::from_variables(variables(pairs))?;
        assert_eq!(settings.cache.root, expected.map(PathBuf::from));
        Ok(())
    }

    #[test]
    fn a_relative_cache_dir_is_refused() -> Result<()> {
        let (settings, refused) = Settings::checked(variables(&[
            (HOME_VAR, "/h"),
            (CACHE_DIR_VAR, "cache"),
        ]))?;
        assert!(
            matches!(
                refused,
                Some(Error::Input(InputError::CacheDirRelative { .. }))
            ),
            "{refused:?}"
        );
        assert!(
            format!("{refused:?}").contains(r#"value: "cache""#),
            "{refused:?}"
        );
        assert_eq!(
            settings.cache.root, None,
            "the cache stays on with a bad variable"
        );
        assert_eq!(settings.cache.roots, [PathBuf::from("/h/cache")]);
        Ok(())
    }

    #[rstest]
    #[case::size(CACHE_MAX_VAR, "lots")]
    #[case::trust(CACHE_TRUST_VAR, "a while")]
    fn a_bad_cache_variable_turns_the_cache_off(
        #[case] name: &'static str,
        #[case] value: &'static str,
    ) -> Result<()> {
        let pairs = [(HOME_VAR, "/h"), (CACHE_DIR_VAR, C), (name, value)];
        let (settings, refused) = Settings::checked(variables(&pairs))?;
        assert!(refused.is_some(), "the bad value was not reported");
        assert_eq!(settings.cache.root, None);
        assert_eq!(
            settings.cache.roots,
            [PathBuf::from("/h/cache"), PathBuf::from(C)]
        );
        assert!(Settings::from_variables(variables(&pairs)).is_ok());
        Ok(())
    }

    #[test]
    fn every_known_root_stays_listed_when_the_cache_is_off() -> Result<()> {
        let off = Settings::from_variables(variables(&[
            (HOME_VAR, "/h"),
            (CACHE_DIR_VAR, C),
            (CACHE_VAR, "off"),
        ]))?;
        assert_eq!(off.cache.root, None);
        assert_eq!(
            off.cache.roots,
            [PathBuf::from("/h/cache"), PathBuf::from(C)]
        );
        assert_eq!(off.cache.clone().off().roots, off.cache.roots);
        let same = Settings::from_variables(variables(&[
            (HOME_VAR, "/h"),
            (CACHE_DIR_VAR, "/h/cache"),
        ]))?;
        assert_eq!(same.cache.roots, [PathBuf::from("/h/cache")]);
        let staging = Settings::from_variables(variables(&[(
            BASE_URL_VAR,
            "http://127.0.0.1:9",
        )]))?;
        assert_eq!(staging.cache.host, "127.0.0.1_9");
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_cache_is_in_library_caches() -> Result<()> {
        let settings = Settings::from_variables(variables(&[("HOME", "/Users/u")]))?;
        assert_eq!(
            settings.cache.root,
            Some(PathBuf::from("/Users/u/Library/Caches/holst"))
        );
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn the_system_cache_follows_xdg() -> Result<()> {
        let xdg = Settings::from_variables(variables(&[
            ("HOME", "/home/u"),
            ("XDG_CACHE_HOME", "/x"),
        ]))?;
        assert_eq!(xdg.cache.root, Some(PathBuf::from("/x/holst")));
        let home = Settings::from_variables(variables(&[("HOME", "/home/u")]))?;
        assert_eq!(home.cache.root, Some(PathBuf::from("/home/u/.cache/holst")));
        Ok(())
    }

    #[test]
    fn the_cache_limit_is_a_size() -> Result<()> {
        let default = Settings::from_variables(variables(&[]))?;
        assert_eq!(default.cache.max_bytes, DEFAULT_CACHE_MAX);
        let set = Settings::from_variables(variables(&[(CACHE_MAX_VAR, "64M")]))?;
        assert_eq!(set.cache.max_bytes, 64 << 20);
        let (_, bad) = Settings::checked(variables(&[(CACHE_MAX_VAR, "lots")]))?;
        assert!(
            matches!(bad, Some(Error::Input(InputError::CacheMaxNotSize { .. }))),
            "{bad:?}"
        );
        assert!(format!("{bad:?}").contains(r#"value: "lots""#), "{bad:?}");
        assert_eq!(set.cache.off().root, None);
        Ok(())
    }

    #[test]
    fn the_cache_trust_is_a_duration() -> Result<()> {
        let default = Settings::from_variables(variables(&[]))?;
        assert_eq!(default.cache.trust_millis, DEFAULT_CACHE_TRUST_MILLIS);
        let set = Settings::from_variables(variables(&[(CACHE_TRUST_VAR, "2h")]))?;
        assert_eq!(set.cache.trust_millis, 2 * 60 * 60 * 1000);
        let never = Settings::from_variables(variables(&[(CACHE_TRUST_VAR, "0")]))?;
        assert_eq!(never.cache.trust_millis, 0);
        let (_, bad) = Settings::checked(variables(&[(CACHE_TRUST_VAR, "a while")]))?;
        assert!(
            matches!(
                bad,
                Some(Error::Input(InputError::CacheTrustNotDuration { .. }))
            ),
            "{bad:?}"
        );
        assert!(
            format!("{bad:?}").contains(r#"value: "a while""#),
            "{bad:?}"
        );
        Ok(())
    }

    #[test]
    fn a_bad_base_url_is_an_error() {
        let settings =
            Settings::from_variables(variables(&[(BASE_URL_VAR, "not a url")]));
        assert!(settings.is_err(), "{settings:?}");
    }
}

/// Where logins live between runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLocation {
    System { home: Option<PathBuf> },
    Dir(PathBuf),
}

impl SessionLocation {
    #[must_use]
    pub fn new(dir: Option<PathBuf>, home: Option<PathBuf>) -> Self {
        dir.filter(|dir| !dir.as_os_str().is_empty())
            .map_or(SessionLocation::System { home }, SessionLocation::Dir)
    }
}

/// The user's home directory from the variables; an empty one counts as none.
#[must_use]
pub fn user_home(read_variable: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    non_empty(read_variable(HOME_DIR_VAR))
}

#[cfg(windows)]
const HOME_DIR_VAR: &str = "USERPROFILE";
#[cfg(not(windows))]
const HOME_DIR_VAR: &str = "HOME";
