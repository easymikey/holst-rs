//! The calls `holst api` may send, and the refusal of the rest.

use std::fmt;

use itertools::Itertools;
use serde::de::DeserializeOwned;

use crate::{effect::http::HttpMethod, remote::api::CATALOGUE};

/// How a call is sent: a read at once, the others only when confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Read,
    Write,
    Admin,
}

/// Whether a call needs a query key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Required,
    Optional,
}

/// The query keys a call takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    Names(&'static [(&'static str, Need)]),
    /// The keys come from an expression, so none is refused.
    Any,
}

/// The body a call takes.
#[derive(Clone, Copy)]
pub enum Body {
    None,
    /// A JSON body or a prepared payload, not checked until the call is typed.
    Json,
    /// A JSON body that must read as the call's body type: [`check`] of it.
    Typed(fn(&str) -> serde_json::Result<()>),
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "None",
            Self::Json => "Json",
            Self::Typed(_) => "Typed",
        })
    }
}

/// Read `text` as a `T`, which is the whole check of a typed body.
///
/// # Errors
/// The serde error, whose text names the field and the position.
pub fn check<T: DeserializeOwned>(text: &str) -> serde_json::Result<()> {
    serde_json::from_str::<T>(text).map(drop)
}

/// One call, with a `/gapi/..` template (`{name}` stands for an id).
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    pub method: HttpMethod,
    pub template: &'static str,
    pub class: Class,
    pub query: Query,
    pub body: Body,
}

impl Query {
    fn accepts(self, key: &str) -> bool {
        match self {
            Self::Names(names) => names.iter().any(|(name, _)| *name == key),
            Self::Any => true,
        }
    }

    fn describe(self) -> String {
        match self {
            Self::Names([]) => "no query".to_owned(),
            Self::Names(names) => {
                format!(
                    "the query {}",
                    names.iter().map(|(name, _)| name).join(", ")
                )
            }
            Self::Any => "any query".to_owned(),
        }
    }
}

/// Why a request is not a catalogue call; nothing was sent.
#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    #[error(
        "holst api sends only the calls holst knows; {method} {path} is not one of them"
    )]
    Unknown { method: HttpMethod, path: String },

    #[error(
        "holst api sends only the calls holst knows; {template} takes {allowed}, not {method}"
    )]
    Method {
        template: &'static str,
        allowed: String,
        method: HttpMethod,
    },

    #[error("\"{raw}\" is not an id segment in {template}")]
    Segment { raw: String, template: &'static str },

    #[error("{method} {template} takes {allowed}; not {key}")]
    Query {
        method: HttpMethod,
        template: &'static str,
        allowed: String,
        key: String,
    },

    #[error("{method} {template} takes no body")]
    NoBody {
        method: HttpMethod,
        template: &'static str,
    },

    #[error("the body does not fit {method} {template}: {error}")]
    Body {
        method: HttpMethod,
        template: &'static str,
        error: serde_json::Error,
    },
}

/// The catalogue entries a request names. Entries that share a method and
/// template all match.
///
/// # Errors
/// [`Refusal`] when the path, method, an id segment or a query key is not in
/// the catalogue.
pub fn find(method: HttpMethod, path: &str) -> Result<Vec<&'static Endpoint>, Refusal> {
    find_in(CATALOGUE, method, path)
}

/// Refuse a body for a call that takes none.
///
/// # Errors
/// [`Refusal::NoBody`] when no entry takes a body.
pub fn check_body(found: &[&Endpoint], has_body: bool) -> Result<(), Refusal> {
    let takes = found
        .iter()
        .any(|endpoint| matches!(endpoint.body, Body::Json | Body::Typed(_)));
    found
        .first()
        .filter(|_| has_body && !takes)
        .map_or(Ok(()), |endpoint| {
            Err(Refusal::NoBody {
                method: endpoint.method,
                template: endpoint.template,
            })
        })
}

fn find_in<'a>(
    catalogue: &[&'a [Endpoint]],
    method: HttpMethod,
    path: &str,
) -> Result<Vec<&'a Endpoint>, Refusal> {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let unknown = || Refusal::Unknown {
        method,
        path: route.to_owned(),
    };
    let segments: Vec<&str> = route
        .strip_prefix("/gapi/")
        .filter(|_| !path.contains('#'))
        .ok_or_else(unknown)?
        .split('/')
        .collect();
    let shaped: Vec<&Endpoint> = catalogue
        .iter()
        .flat_map(|list| list.iter())
        .filter(|endpoint| fits(endpoint.template, &segments))
        .collect();
    let first = shaped.first().ok_or_else(unknown)?;
    let sent: Vec<&Endpoint> = shaped
        .iter()
        .copied()
        .filter(|endpoint| endpoint.method == method)
        .collect();
    let named = sent.first().ok_or_else(|| Refusal::Method {
        template: first.template,
        allowed: shaped
            .iter()
            .map(|endpoint| endpoint.method.as_str())
            .sorted()
            .dedup()
            .join(" or "),
        method,
    })?;
    let valid: Vec<&Endpoint> = sent
        .iter()
        .copied()
        .filter(|endpoint| misfit(endpoint.template, &segments).is_none())
        .collect();
    valid.first().ok_or_else(|| Refusal::Segment {
        raw: misfit(named.template, &segments)
            .unwrap_or_default()
            .to_owned(),
        template: named.template,
    })?;
    let rejected = query
        .split('&')
        .map(|pair| pair.split_once('=').map_or(pair, |(key, _)| key))
        .filter(|key| !key.is_empty())
        .find(|key| !valid.iter().any(|endpoint| endpoint.query.accepts(key)));
    match (rejected, valid.first()) {
        (Some(key), Some(endpoint)) => Err(Refusal::Query {
            method,
            template: endpoint.template,
            allowed: endpoint.query.describe(),
            key: key.to_owned(),
        }),
        (None | Some(_), _) => Ok(valid),
    }
}

fn template_segments(template: &str) -> Vec<&str> {
    template
        .strip_prefix("/gapi/")
        .unwrap_or(template)
        .split('/')
        .collect()
}

fn is_placeholder(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

/// The same number of segments, and every literal equal.
fn fits(template: &str, path: &[&str]) -> bool {
    let wanted = template_segments(template);
    wanted.len() == path.len()
        && wanted
            .iter()
            .zip(path)
            .all(|(want, got)| is_placeholder(want) || want == got)
}

/// The first path segment standing for an id that is not one.
fn misfit<'p>(template: &str, path: &[&'p str]) -> Option<&'p str> {
    template_segments(template)
        .iter()
        .zip(path)
        .filter(|(want, _)| is_placeholder(want))
        .map(|(_, got)| *got)
        .find(|got| !is_id_segment(got))
}

/// `[A-Za-z0-9._~-]+` other than `.` and `..`: a `%` never gets through, so an
/// encoded `/` or `..` cannot reach another route after the server decodes it.
fn is_id_segment(segment: &str) -> bool {
    !matches!(segment, "" | "." | "..")
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(&byte))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde::Deserialize;

    use super::{
        Body::{self, Json},
        Class::{self, Admin, Read, Write},
        Endpoint,
        Need::{Optional, Required},
        Query::{self, Names},
        check,
        check_body,
        find_in,
        fits,
        is_id_segment,
        template_segments,
    };
    use crate::{effect::http::HttpMethod, remote::api::CATALOGUE};

    const fn call(
        method: HttpMethod,
        template: &'static str,
        class: Class,
        query: Query,
        body: Body,
    ) -> Endpoint {
        Endpoint {
            method,
            template,
            class,
            query,
            body,
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    #[expect(dead_code, reason = "read only by the check")]
    struct SetSample {
        channel_id: String,
    }

    const CHANNEL: &str = "/gapi/channel/{channel_id}";

    const SAMPLE: &[Endpoint] = &[
        call(
            HttpMethod::Get,
            "/gapi/recent/channels/page",
            Read,
            Names(&[("pageSize", Required), ("cursor", Optional)]),
            Body::None,
        ),
        call(HttpMethod::Get, CHANNEL, Read, Names(&[]), Body::None),
        call(HttpMethod::Patch, CHANNEL, Write, Names(&[]), Json),
        call(HttpMethod::Patch, CHANNEL, Write, Names(&[]), Json),
        call(
            HttpMethod::Post,
            "/gapi/channel/{channel_id}/leave",
            Write,
            Names(&[]),
            Body::None,
        ),
        call(
            HttpMethod::Post,
            "/gapi/recent/set",
            Write,
            Names(&[]),
            Body::Typed(check::<SetSample>),
        ),
    ];

    fn found(method: HttpMethod, path: &str) -> Option<usize> {
        find_in(&[SAMPLE], method, path).ok().map(|list| list.len())
    }

    fn refusal(method: HttpMethod, path: &str) -> Option<String> {
        find_in(&[SAMPLE], method, path)
            .err()
            .map(|refusal| refusal.to_string())
    }

    #[rstest]
    #[case::plain_get(HttpMethod::Get, "/gapi/channel/abc", Some(1))]
    #[case::same_method_and_template_count_together(
        HttpMethod::Patch,
        "/gapi/channel/abc",
        Some(2)
    )]
    #[case::every_id_character(HttpMethod::Get, "/gapi/channel/AZaz09._~-", Some(1))]
    #[case::literal_segments(HttpMethod::Post, "/gapi/channel/abc/leave", Some(1))]
    #[case::both_keys(
        HttpMethod::Get,
        "/gapi/recent/channels/page?pageSize=3&cursor=c",
        Some(1)
    )]
    #[case::optional_key_left_out(
        HttpMethod::Get,
        "/gapi/recent/channels/page?pageSize=3",
        Some(1)
    )]
    #[case::empty_query(HttpMethod::Get, "/gapi/channel/abc?", Some(1))]
    #[case::key_without_value(
        HttpMethod::Get,
        "/gapi/recent/channels/page?cursor&&pageSize=1",
        Some(1)
    )]
    fn known_calls_are_found(
        #[case] method: HttpMethod,
        #[case] path: &str,
        #[case] count: Option<usize>,
    ) {
        assert_eq!(found(method, path), count);
    }

    #[rstest]
    #[case::unknown_path(
        HttpMethod::Get,
        "/gapi/x",
        "holst api sends only the calls holst knows; GET /gapi/x is not one of them"
    )]
    #[case::outside_gapi(
        HttpMethod::Get,
        "/other/channel/abc",
        "holst api sends only the calls holst knows; GET /other/channel/abc is not one of them"
    )]
    #[case::fragment(
        HttpMethod::Get,
        "/gapi/channel/abc#top",
        "holst api sends only the calls holst knows; GET /gapi/channel/abc#top is not one of them"
    )]
    #[case::fragment_after_query(
        HttpMethod::Get,
        "/gapi/channel/abc?a=b#top",
        "holst api sends only the calls holst knows; GET /gapi/channel/abc is not one of them"
    )]
    #[case::trailing_slash(
        HttpMethod::Get,
        "/gapi/channel/abc/",
        "holst api sends only the calls holst knows; GET /gapi/channel/abc/ is not one of them"
    )]
    #[case::longer_than_any_call(
        HttpMethod::Get,
        "/gapi/channel/abc/leave/now",
        "holst api sends only the calls holst knows; GET /gapi/channel/abc/leave/now is not one of them"
    )]
    #[case::wrong_method(
        HttpMethod::Delete,
        "/gapi/channel/abc",
        "holst api sends only the calls holst knows; /gapi/channel/{channel_id} takes GET or PATCH, not DELETE"
    )]
    #[case::method_of_a_literal_route(
        HttpMethod::Get,
        "/gapi/channel/abc/leave",
        "holst api sends only the calls holst knows; /gapi/channel/{channel_id}/leave takes POST, not GET"
    )]
    #[case::encoded_slash_and_dots(
        HttpMethod::Get,
        "/gapi/channel/a%2F..",
        "\"a%2F..\" is not an id segment in /gapi/channel/{channel_id}"
    )]
    #[case::single_dot(
        HttpMethod::Get,
        "/gapi/channel/.",
        "\".\" is not an id segment in /gapi/channel/{channel_id}"
    )]
    #[case::double_dot(
        HttpMethod::Post,
        "/gapi/channel/../leave",
        "\"..\" is not an id segment in /gapi/channel/{channel_id}/leave"
    )]
    #[case::empty_segment(
        HttpMethod::Get,
        "/gapi/channel/",
        "\"\" is not an id segment in /gapi/channel/{channel_id}"
    )]
    #[case::space(
        HttpMethod::Get,
        "/gapi/channel/a b",
        "\"a b\" is not an id segment in /gapi/channel/{channel_id}"
    )]
    #[case::unknown_query_key(
        HttpMethod::Get,
        "/gapi/recent/channels/page?pageSize=3&size=3",
        "GET /gapi/recent/channels/page takes the query pageSize, cursor; not size"
    )]
    #[case::query_on_a_call_without(
        HttpMethod::Get,
        "/gapi/channel/abc?x=1",
        "GET /gapi/channel/{channel_id} takes no query; not x"
    )]
    fn refusals_say_why(
        #[case] method: HttpMethod,
        #[case] path: &str,
        #[case] text: &str,
    ) {
        assert_eq!(refusal(method, path).as_deref(), Some(text));
    }

    #[rstest]
    #[case::fits(r#"{"channelId":"c"}"#, None)]
    #[case::unknown_field(
        r#"{"channel":"c"}"#,
        Some("unknown field `channel`, expected `channelId` at line 1 column 10")
    )]
    #[case::missing_field("{}", Some("missing field `channelId` at line 1 column 2"))]
    #[case::wrong_type(
        r#"{"channelId":1}"#,
        Some("invalid type: integer `1`, expected a string at line 1 column 14")
    )]
    #[case::not_json("{", Some("EOF while parsing an object at line 1 column 1"))]
    fn a_typed_body_is_read_as_its_type(
        #[case] body: &str,
        #[case] text: Option<&str>,
    ) {
        let error = check::<SetSample>(body)
            .err()
            .map(|error| error.to_string());
        assert_eq!(error.as_deref(), text);
    }

    #[test]
    fn a_typed_call_takes_a_body() {
        let refused = find_in(&[SAMPLE], HttpMethod::Post, "/gapi/recent/set")
            .ok()
            .and_then(|list| check_body(&list, true).err());
        assert!(refused.is_none(), "{refused:?}");
    }

    #[rstest]
    #[case::no_body_for_a_call_without(
        HttpMethod::Post,
        "/gapi/channel/abc/leave",
        true,
        Some("POST /gapi/channel/{channel_id}/leave takes no body")
    )]
    #[case::no_body_sent(HttpMethod::Post, "/gapi/channel/abc/leave", false, None)]
    #[case::body_for_a_call_with(HttpMethod::Patch, "/gapi/channel/abc", true, None)]
    fn a_body_is_refused_where_none_is_taken(
        #[case] method: HttpMethod,
        #[case] path: &str,
        #[case] has_body: bool,
        #[case] text: Option<&str>,
    ) {
        let refused = find_in(&[SAMPLE], method, path)
            .ok()
            .and_then(|list| check_body(&list, has_body).err())
            .map(|refusal| refusal.to_string());
        assert_eq!(refused.as_deref(), text);
    }

    #[rstest]
    #[case::letters("abc", true)]
    #[case::all_marks("a.b_c~d-e", true)]
    #[case::empty("", false)]
    #[case::dot(".", false)]
    #[case::dots("..", false)]
    #[case::dots_inside("a..b", true)]
    #[case::percent("a%2Fb", false)]
    #[case::slash("a/b", false)]
    #[case::question("a?b", false)]
    #[case::hash("a#b", false)]
    #[case::non_ascii("é", false)]
    fn id_segment_rule(#[case] segment: &str, #[case] valid: bool) {
        assert_eq!(is_id_segment(segment), valid);
    }

    fn money_or_login(entries: &[Endpoint]) -> Vec<&'static str> {
        const WORDS: [&str; 9] = [
            "subscription",
            "billing",
            "payment",
            "card",
            "invoice",
            "checkout",
            "login",
            "logout",
            "password",
        ];
        entries
            .iter()
            .map(|endpoint| endpoint.template)
            .filter(|template| WORDS.iter().any(|word| template.contains(word)))
            .collect()
    }

    /// Pairs of entries with one method where one template matches the other
    /// and the classes differ.
    fn class_conflicts(entries: &[Endpoint]) -> Vec<(&'static str, &'static str)> {
        entries
            .iter()
            .flat_map(|one| entries.iter().map(move |other| (one, other)))
            .filter(|(one, other)| {
                one.method == other.method
                    && one.class != other.class
                    && fits(one.template, &template_segments(other.template))
            })
            .map(|(one, other)| (one.template, other.template))
            .collect()
    }

    fn catalogue() -> Vec<Endpoint> {
        CATALOGUE
            .iter()
            .flat_map(|list| list.iter().copied())
            .collect()
    }

    #[test]
    fn the_catalogue_holds_no_money_or_login_call() {
        assert_eq!(money_or_login(&catalogue()), Vec::<&str>::new());
    }

    #[test]
    fn no_write_or_admin_call_takes_an_untyped_body() {
        let untyped: Vec<_> = catalogue()
            .into_iter()
            .filter(|entry| matches!(entry.class, Write | Admin))
            .filter(|entry| matches!(entry.body, Json))
            .map(|entry| entry.template)
            .collect();
        assert_eq!(untyped, Vec::<&str>::new());
    }

    #[test]
    fn money_and_login_paths_are_found_by_name() {
        let entries = [
            call(
                HttpMethod::Get,
                "/gapi/subscription/get",
                Read,
                Names(&[]),
                Body::None,
            ),
            call(
                HttpMethod::Post,
                "/gapi/login/with_email",
                Write,
                Names(&[]),
                Body::None,
            ),
            call(
                HttpMethod::Get,
                "/gapi/current_user",
                Read,
                Names(&[]),
                Body::None,
            ),
        ];
        assert_eq!(
            money_or_login(&entries),
            ["/gapi/subscription/get", "/gapi/login/with_email"]
        );
    }

    #[test]
    fn overlapping_entries_agree_on_class() {
        assert_eq!(class_conflicts(&catalogue()), []);
    }

    #[test]
    fn a_literal_route_inside_an_id_route_must_agree_on_class() {
        let entries = [
            call(
                HttpMethod::Delete,
                "/gapi/channel/{channel_id}",
                Write,
                Names(&[]),
                Body::None,
            ),
            call(
                HttpMethod::Delete,
                "/gapi/channel/all",
                Admin,
                Names(&[]),
                Body::None,
            ),
            call(
                HttpMethod::Get,
                "/gapi/channel/all",
                Read,
                Names(&[]),
                Body::None,
            ),
        ];
        assert_eq!(
            class_conflicts(&entries),
            [("/gapi/channel/{channel_id}", "/gapi/channel/all")]
        );
    }

    #[test]
    fn every_entry_is_listed_in_the_snapshot() {
        let lines = catalogue()
            .iter()
            .map(|endpoint| {
                format!(
                    "{} {} {:?} {:?} {:?}",
                    endpoint.method,
                    endpoint.template,
                    endpoint.class,
                    endpoint.query,
                    endpoint.body
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(lines, @r#"
        POST /gapi/recent/set Write Names([]) Typed
        GET /gapi/recent/channels/page Read Names([("pageSize", Required), ("cursor", Optional)]) None
        GET /gapi/favorite_channel/channels/{workspace_id} Read Names([]) None
        POST /gapi/favorite_channel/add Write Names([]) Typed
        DELETE /gapi/favorite_channel/delete Write Names([]) Typed
        GET /gapi/comment/thread_list/{channel_id} Read Names([("channelId", Required)]) None
        POST /gapi/comment/set_read_at Write Names([]) Typed
        GET /gapi/channel/{channel_id} Read Names([]) None
        GET /gapi/channel_user/users Read Names([("channel_id", Required)]) None
        GET /gapi/channel/{channel_id}/parent_total_permits Read Names([]) None
        GET /gapi/channel_user_order_index/{workspace_id}/list Read Names([]) None
        PATCH /gapi/channel/{channel_id} Admin Names([]) Typed
        PATCH /gapi/channel/{channel_id} Admin Names([]) Typed
        PATCH /gapi/channel/{channel_id} Admin Names([]) Typed
        PATCH /gapi/channel/{channel_id} Admin Names([]) Typed
        POST /gapi/channel/{channel_id}/delete Write Names([]) Typed
        POST /gapi/channel/restore/{channel_id} Write Names([]) Typed
        POST /gapi/channel/{channel_id}/move Write Names([]) Typed
        POST /gapi/channel/{channel_id}/leave Write Names([]) None
        POST /gapi/channel/create Write Names([]) Typed
        POST /gapi/channel/duplicate Write Names([]) Typed
        PATCH /gapi/channel/{channel_id} Admin Names([]) Typed
        PATCH /gapi/channel/{channel_id}/sharing_settings Admin Names([]) Typed
        POST /gapi/channel_user/set Admin Names([]) Typed
        POST /gapi/channel/{channel_id}/transfer_ownership Admin Names([]) Typed
        GET /gapi/backup/list Read Names([("name", Required)]) None
        GET /gapi/board_template/categories/standard Read Names([]) None
        GET /gapi/board_template/categories/{category}/standard_templates Read Names([]) None
        GET /gapi/board_template/personal Read Names([]) None
        GET /gapi/board_template/workspace/{workspace_id} Read Names([]) None
        GET /gapi/board_template/get Read Names([("board_template_id", Required)]) None
        DELETE /gapi/board_template/delete Write Names([("board_template_id", Required)]) None
        POST /gapi/board_template/create Write Names([]) Typed
        PATCH /gapi/board_template/update Write Names([("board_template_id", Required)]) Typed
        GET /gapi/current_user Read Names([]) None
        GET /gapi/current_user/joinable_workspaces Read Names([]) None
        GET /gapi/personal_access_token/list Read Names([("statuses", Required)]) None
        GET /gapi/link_preview Read Names([("link", Required)]) None
        GET /gapi/workspace/{workspace_id} Read Names([]) None
        GET /gapi/workspace/{workspace_id}/trash Read Names([]) None
        GET /gapi/workspace_user/{workspace_id}/list_last_active_dates Admin Names([]) None
        GET /gapi/workspace/{workspace_id}/created_channels_count Read Names([("userId", Required)]) None
        GET /gapi/workspace/{workspace_id}/sso_settings Admin Names([]) None
        GET /gapi/workspace_domains/{workspace_id}/list Admin Names([]) None
        PATCH /gapi/workspace/{workspace_id} Admin Names([]) Typed
        POST /gapi/workspace/create Write Names([]) Typed
        GET /gapi/workspace/{workspace_id}/tree Read Names([]) None
        GET /gapi/audit_events Admin Any None
        "#);
    }
}
