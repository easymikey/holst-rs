//! The entries of [`CATALOGUE`](crate::remote::api::CATALOGUE).

use std::fmt;

use serde::de::DeserializeOwned;

use crate::effect::http::HttpMethod;

/// What a call does: read, write, or change access or workspace settings.
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

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde::Deserialize;

    use super::{
        Body::{self, Json},
        Class::{self, Admin, Read, Write},
        Endpoint,
        Query::{self, Names},
        check,
    };
    use crate::{effect::http::HttpMethod, remote::api::CATALOGUE};

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
        GET /gapi/session/list Read Names([]) None
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
