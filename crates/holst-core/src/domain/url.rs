//! Addresses safe for the session: base, request paths, path segments.

use std::{borrow::Cow, str::FromStr};

use derive_more::{AsRef, Display};
use reqwest::Url;

use crate::error::{Error, Result, input::InputError};

pub const DEFAULT_BASE: &str = "https://app.holst.so";

/// A checked API base address; only made by parsing.
///
/// ```
/// use holst_core::domain::url::BaseUrl;
///
/// let base: BaseUrl = "https://app.holst.so/".parse()?;
/// assert_eq!(base.as_str(), "https://app.holst.so");
/// assert!("http://app.holst.so".parse::<BaseUrl>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Debug, Clone, Display)]
#[display("{raw}")]
pub struct BaseUrl {
    pub(crate) raw: String,
    pub(crate) url: Url,
}

impl FromStr for BaseUrl {
    type Err = Error;

    /// An https origin; plain http only on localhost, for a local mock.
    ///
    /// # Errors
    ///
    /// [`crate::error::input::InputError::BaseNotUrl`],
    /// [`crate::error::input::InputError::BaseNotHttps`] or
    /// [`crate::error::input::InputError::BaseNotOrigin`].
    fn from_str(raw: &str) -> Result<Self> {
        let raw = raw.trim_end_matches('/').to_owned();
        Url::parse(&raw)
            .map_err(|_| Error::Input(InputError::BaseNotUrl { base: raw.clone() }))
            .and_then(|url| {
                require(url, is_secure, || {
                    Error::Input(InputError::BaseNotHttps { base: raw.clone() })
                })
            })
            .and_then(|url| {
                require(url, is_origin, || {
                    Error::Input(InputError::BaseNotOrigin { base: raw.clone() })
                })
            })
            .map(|url| Self { raw, url })
    }
}

impl BaseUrl {
    /// `raw` if given, else [`DEFAULT_BASE`].
    ///
    /// # Errors
    /// As [`BaseUrl::from_str`].
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    ///
    /// assert_eq!(BaseUrl::or_default(None)?.as_str(), "https://app.holst.so");
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    pub fn or_default(raw: Option<&str>) -> Result<Self> {
        raw.unwrap_or(DEFAULT_BASE).parse()
    }

    /// The address as text, without a trailing `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Link to a board in the web client on the same address as the API.
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    ///
    /// let base: BaseUrl = "https://holst.example".parse()?;
    /// assert_eq!(base.board_link("b1"), "https://holst.example/board/b1");
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    #[must_use]
    pub fn board_link(&self, id: &str) -> String {
        let base = &self.raw;
        format!("{base}/board/{}", percent_encode(id))
    }

    /// Link to a folder in the web client on the same address as the API.
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    /// let base: BaseUrl = "https://holst.example".parse()?;
    /// assert_eq!(
    ///     base.folder_link("w1", "f1"),
    ///     "https://holst.example/w/w1/f/f1"
    /// );
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    #[must_use]
    pub fn folder_link(&self, workspace: &str, id: &str) -> String {
        let base = &self.raw;
        format!(
            "{base}/w/{}/f/{}",
            percent_encode(workspace),
            percent_encode(id)
        )
    }
    /// Link to one object on a board: the web client opens the board there; ids
    /// are `%XX`-encoded.
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    ///
    /// let base: BaseUrl = "https://app.holst.so".parse()?;
    /// assert_eq!(
    ///     base.object_link("b1", "o1"),
    ///     "https://app.holst.so/board/b1?objectId=o1"
    /// );
    /// assert_eq!(
    ///     base.object_link("b1", "o&x=1#y"),
    ///     "https://app.holst.so/board/b1?objectId=o%26x%3D1%23y"
    /// );
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    #[must_use]
    pub fn object_link(&self, board: &str, object: &str) -> String {
        let board_url = self.board_link(board);
        format!("{board_url}?objectId={}", percent_encode(object))
    }

    /// Link to an object on a board, or to the board without one.
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    ///
    /// let base: BaseUrl = "https://app.holst.so".parse()?;
    /// assert_eq!(base.link("b1", None), "https://app.holst.so/board/b1");
    /// assert_eq!(
    ///     base.link("b1", Some("o1")),
    ///     "https://app.holst.so/board/b1?objectId=o1"
    /// );
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    #[must_use]
    pub fn link(&self, board: &str, object: Option<&str>) -> String {
        object.map_or_else(
            || self.board_link(board),
            |object| self.object_link(board, object),
        )
    }

    /// The host and a non-default port as one safe directory name.
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    ///
    /// let base: BaseUrl = "https://App.Holst.so".parse()?;
    /// assert_eq!(base.host_key(), "app.holst.so");
    /// let local: BaseUrl = "http://127.0.0.1:8080".parse()?;
    /// assert_eq!(local.host_key(), "127.0.0.1_8080");
    /// let six: BaseUrl = "http://[::1]:9".parse()?;
    /// assert_eq!(six.host_key(), "---1-_9");
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    #[must_use]
    pub fn host_key(&self) -> String {
        let host = self.url.host_str().unwrap_or_default().to_ascii_lowercase();
        let port = self.url.port().map(|port| format!("_{port}"));
        let safe: String = host
            .chars()
            .map(|char| match char {
                'a'..='z' | '0'..='9' | '.' | '-' => char,
                _ => '-',
            })
            .collect();
        format!("{safe}{}", port.unwrap_or_default())
    }

    /// The full URL of a request; absolute addresses only on the base's origin.
    ///
    /// # Errors
    ///
    /// [`crate::error::input::InputError::NotUrl`],
    /// [`crate::error::input::InputError::ForeignOrigin`]
    /// or [`crate::error::input::InputError::NotApiPath`].
    ///
    /// ```
    /// use holst_core::domain::url::BaseUrl;
    ///
    /// let base: BaseUrl = "https://app.holst.so".parse()?;
    /// assert_eq!(
    ///     base.url_for("/gapi/x")?.as_str(),
    ///     "https://app.holst.so/gapi/x"
    /// );
    /// assert!(base.url_for("https://evil.test/gapi/x").is_err());
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    pub fn url_for(&self, path: &str) -> Result<Url> {
        if has_scheme(path) {
            self.absolute(path)
        } else {
            self.relative(path)
        }
    }

    fn relative(&self, path: &str) -> Result<Url> {
        let slash = path.strip_prefix('/').map_or("/", |_| "");
        Url::parse(&format!("{}{slash}{path}", self.raw)).map_err(|_| {
            Error::Input(InputError::NotApiPath {
                path: path.to_owned(),
            })
        })
    }

    fn absolute(&self, path: &str) -> Result<Url> {
        let url = Url::parse(path).map_err(|_| {
            Error::Input(InputError::NotUrl {
                url: path.to_owned(),
            })
        })?;
        let origin = url.origin();
        (origin == self.url.origin()).then_some(url).ok_or_else(|| {
            Error::Input(InputError::ForeignOrigin {
                origin: origin.ascii_serialization(),
                base: self.raw.clone(),
            })
        })
    }
}

fn has_scheme(path: &str) -> bool {
    path.split_once("://").is_some_and(|(scheme, _)| {
        scheme
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    })
}

fn require(
    url: Url,
    ok: fn(&Url) -> bool,
    error: impl FnOnce() -> Error,
) -> Result<Url> {
    if ok(&url) { Ok(url) } else { Err(error()) }
}

fn is_origin(url: &Url) -> bool {
    url.path() == "/" && url.query().is_none() && url.fragment().is_none()
}

fn is_secure(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        _ => false,
    }
}

/// One path segment, `%XX`-encoded; `.`, `..` and `""` are refused.
///
/// # Errors
///
/// [`crate::error::input::InputError::NotPathSegment`] for `""`, `.` and `..`.
///
/// ```
/// use holst_core::domain::url::encode_segment;
///
/// assert_eq!(encode_segment("a/b")?, "a%2Fb");
/// assert!(encode_segment("..").is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
pub fn encode_segment(raw: &str) -> Result<Cow<'_, str>> {
    match raw {
        "" | "." | ".." => Err(Error::Input(InputError::NotPathSegment {
            raw: raw.to_owned(),
        })),
        any => Ok(percent_encode(any)),
    }
}

/// Text with every byte but the RFC 3986 unreserved ones `%XX`-encoded, for a
/// path segment or a query value.
///
/// ```
/// use holst_core::domain::url::percent_encode;
///
/// assert_eq!(percent_encode("a&b c#d"), "a%26b%20c%23d");
/// assert_eq!(percent_encode("o-1_x.y~"), "o-1_x.y~");
/// ```
#[must_use]
pub fn percent_encode(raw: &str) -> Cow<'_, str> {
    match raw {
        clean if clean.bytes().all(unreserved) => Cow::Borrowed(clean),
        dirty => Cow::Owned(dirty.bytes().fold(
            String::with_capacity(dirty.len() * 3),
            |mut out, b| {
                match b {
                    b if unreserved(b) => out.push(char::from(b)),
                    b => out.extend(['%', hex(b >> 4), hex(b & 0xF)]),
                }
                out
            },
        )),
    }
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn hex(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).map_or('0', |c| c.to_ascii_uppercase())
}

/// An API path from the root, without scheme or host; only made by parsing.
///
/// ```
/// use holst_core::domain::url::ApiPath;
///
/// let path: ApiPath = "/gapi/current_user".parse()?;
/// assert_eq!(path.as_str(), "/gapi/current_user");
/// assert!("gapi/current_user".parse::<ApiPath>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Display, AsRef)]
#[as_ref(str)]
pub struct ApiPath(String);

impl ApiPath {
    /// The path as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ApiPath {
    type Err = Error;

    /// # Errors
    ///
    /// [`crate::error::input::InputError::FullUrl`] or
    /// [`crate::error::input::InputError::RelativePath`].
    fn from_str(path: &str) -> Result<Self> {
        match path {
            full if has_scheme(full) || full.starts_with("//") => {
                Err(Error::Input(InputError::FullUrl))
            }
            rooted if rooted.starts_with('/') => Ok(Self(rooted.to_owned())),
            _ => Err(Error::Input(InputError::RelativePath)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use reqwest::Url;
    use rstest::rstest;

    use crate::{
        domain::{
            id::BoardId,
            url::{ApiPath, BaseUrl, DEFAULT_BASE, encode_segment},
        },
        error::{Error, input::InputError},
    };

    const ID: &str = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b";

    fn base() -> Option<BaseUrl> {
        DEFAULT_BASE.parse().ok()
    }

    #[rstest]
    #[case::default(DEFAULT_BASE, "https://app.holst.so/board/")]
    #[case::custom_base("https://holst.example/", "https://holst.example/board/")]
    fn board_url_follows_base_and_round_trips(#[case] raw: &str, #[case] prefix: &str) {
        let url = raw.parse::<BaseUrl>().map(|base| base.board_link(ID));
        let expected = format!("{prefix}{ID}");
        assert_eq!(url.as_deref().ok(), Some(expected.as_str()));
        assert_eq!(
            url.ok()
                .as_deref()
                .and_then(BoardId::find_in)
                .map(String::from)
                .as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn object_links_encode_ids_from_the_board() {
        let base = DEFAULT_BASE.parse::<BaseUrl>().ok();
        let link = base.as_ref().map(|b| b.object_link(ID, "x&y=1 #\u{1b}"));
        assert_eq!(
            link.as_deref(),
            Some(
                format!("https://app.holst.so/board/{ID}?objectId=x%26y%3D1%20%23%1B")
                    .as_str()
            )
        );
    }

    #[rstest]
    #[case::foreign_host("https://evil.test/gapi/current_user", None)]
    #[case::plain_http("http://app.holst.so/gapi/current_user", None)]
    #[case::lookalike_host("https://app.holst.so.evil.test/x", None)]
    #[case::other_port("https://app.holst.so:8443/x", None)]
    #[case::own_absolute(
        "https://app.holst.so/gapi/current_user",
        Some("https://app.holst.so/gapi/current_user")
    )]
    #[case::relative("gapi/x", Some("https://app.holst.so/gapi/x"))]
    #[case::url_in_query(
        "/gapi/link_preview?url=https://x.test/a",
        Some("https://app.holst.so/gapi/link_preview?url=https://x.test/a")
    )]
    fn only_own_host_is_reachable(#[case] path: &str, #[case] expected: Option<&str>) {
        let url = base().and_then(|base| base.url_for(path).ok());
        assert_eq!(url.as_ref().map(Url::as_str), expected);
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Verdict {
        Accepted,
        Rejected,
    }

    #[rstest]
    #[case::plain_http("http://app.holst.so", Verdict::Rejected)]
    #[case::ftp("ftp://app.holst.so", Verdict::Rejected)]
    #[case::not_a_url("app.holst.so", Verdict::Rejected)]
    #[case::loopback_ip("http://127.0.0.1:8080", Verdict::Accepted)]
    #[case::localhost("http://localhost:1", Verdict::Accepted)]
    #[case::ipv6_loopback("http://[::1]:1", Verdict::Accepted)]
    #[case::trailing_slash("https://app.holst.so/", Verdict::Accepted)]
    #[case::with_port("https://app.holst.so:8443", Verdict::Accepted)]
    #[case::unicode_host("https://пример.рф", Verdict::Rejected)]
    #[case::punycode_host("https://xn--e1afmkfd.xn--p1ai", Verdict::Accepted)]
    fn plain_http_base_only_for_localhost(
        #[case] raw: &str,
        #[case] accepted: Verdict,
    ) {
        assert_eq!(
            raw.parse::<BaseUrl>().is_ok(),
            accepted == Verdict::Accepted
        );
    }

    #[rstest]
    #[case::query("https://app.holst.so?x=1")]
    #[case::fragment("https://app.holst.so#top")]
    #[case::path("https://app.holst.so/api")]
    #[case::path_and_slash("https://app.holst.so/api/")]
    fn base_is_only_an_origin(#[case] raw: &str) {
        let error = raw.parse::<BaseUrl>().err();
        assert!(
            matches!(error, Some(Error::Input(InputError::BaseNotOrigin { .. }))),
            "{error:?}"
        );
    }

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        let base = "https://example.test/"
            .parse::<BaseUrl>()
            .map(|b| b.to_string());
        assert_eq!(base.ok().as_deref(), Some("https://example.test"));
    }

    #[test]
    fn api_path_keeps_the_path() {
        let path = "/gapi/current_user"
            .parse::<ApiPath>()
            .map(|api_path| api_path.to_string());
        assert_eq!(path.ok().as_deref(), Some("/gapi/current_user"));
    }

    #[rstest]
    #[case::clean("abc-1.2_~", Some("abc-1.2_~"))]
    #[case::slash_and_query("a/b?c#d", Some("a%2Fb%3Fc%23d"))]
    #[case::cyrillic("я", Some("%D1%8F"))]
    #[case::percent("50%", Some("50%25"))]
    #[case::empty("", None)]
    #[case::dot(".", None)]
    #[case::dot_dot("..", None)]
    fn segment_encodes_everything_but_unreserved(
        #[case] raw: &str,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(encode_segment(raw).ok().as_deref(), expected);
    }

    #[test]
    fn clean_segment_is_borrowed() {
        assert!(matches!(encode_segment("abc"), Ok(Cow::Borrowed("abc"))));
    }

    const FULL_URL: &str =
        "expected a path, not a full URL: the session is sent only to its own server";
    const NOT_FROM_ROOT: &str =
        "the path must start with \"/\", for example /gapi/current_user";

    #[rstest]
    #[case::full_url("https://evil.test/gapi", Some(FULL_URL))]
    #[case::scheme_relative("//evil.test/gapi", Some(FULL_URL))]
    #[case::other_scheme("ftp://evil.test/gapi", Some(FULL_URL))]
    #[case::url_in_query("/gapi/link_preview?url=https://x.test", None)]
    #[case::url_in_fragment("/gapi/x#https://x.test", None)]
    #[case::not_from_root("gapi/current_user", Some(NOT_FROM_ROOT))]
    #[case::empty("", Some(NOT_FROM_ROOT))]
    #[case::from_root("/gapi/current_user", None)]
    fn absolute_urls_are_rejected_to_protect_the_session(
        #[case] path: &str,
        #[case] error: Option<&str>,
    ) {
        let got = path
            .parse::<ApiPath>()
            .err()
            .map(|parse_error| parse_error.to_string());
        assert_eq!(got.as_deref(), error);
    }
}
