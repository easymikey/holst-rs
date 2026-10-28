//! The client on the real transport against a local mock (no Holst).

use std::time::Duration;

use holst_core::{
    Error,
    domain::{node::DeleteMode, session::Session},
    effect::{
        http::{HttpBody, HttpMethod},
        request::{ApiRequest, Authorization},
    },
    error::{hint::Hint, http::HttpError, input::InputError, login::LoginError},
};
use rstest::rstest;
use serde_json::Value;

use crate::common;

type TestResult = holst_core::Result<()>;

fn raw(method: HttpMethod, path: &str, body: Option<&Value>) -> ApiRequest {
    ApiRequest {
        method,
        path: path.to_owned(),
        query: Vec::new(),
        body: body.and_then(|value| HttpBody::json(value).ok()),
        authorization: Authorization::Session,
    }
}

const TOKEN: &str = "tok-SECRET-42";

#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Redirect(String),
    Status { status: u16, hint: Option<Hint> },
    Revoked { hint: Option<Hint> },
    Other(String),
}

impl Refused {
    fn of(err: Option<Error>) -> Self {
        match err {
            Some(Error::Http(HttpError::ForeignRedirect { location })) => {
                Refused::Redirect(location)
            }
            Some(err @ Error::Login(LoginError::SessionRevoked)) => {
                Refused::Revoked { hint: err.hint() }
            }
            Some(err @ Error::Http(HttpError::Status { .. })) => Refused::Status {
                status: err.status().unwrap_or_default(),
                hint: err.hint(),
            },
            other => Refused::Other(format!("{other:?}")),
        }
    }
}

#[rstest]
#[case::foreign_redirect(
    302,
    "location: http://127.0.0.1:9/steal\r\n",
    "",
    Refused::Redirect("http://127.0.0.1:9".into())
)]
// The reply clears the cookie, yet it is an expired login, not a bare token.
#[case::expired_cookie_cleared(
    401,
    "set-cookie: sid=; Max-Age=0\r\n",
    "",
    Refused::Revoked {
        hint: Some(Hint::LoginAgain)
    }
)]
#[tokio::test]
async fn single_reply_ends_the_request(
    #[case] status: u16,
    #[case] headers: &'static str,
    #[case] body: &'static str,
    #[case] expected: Refused,
) -> TestResult {
    let mock = common::script(vec![(status, headers, body)]).await?;
    let err = mock
        .client(TOKEN)?
        .send_json(raw(HttpMethod::Get, "/gapi/current_user", None))
        .await
        .err();
    assert_eq!(Refused::of(err), expected);
    assert_eq!(mock.hits(), 1);
    Ok(())
}

#[tokio::test]
async fn foreign_host_gets_no_request() -> TestResult {
    let mock = common::script(vec![(200, "", "{}")]).await?;
    let c = holst_core::command::login::connect(
        Some(Session::new("tok".into())),
        "https://app.holst.so".parse()?,
        holst_core::system::clock::SystemClock,
        holst_core::system::id::RandomIds,
    )?;
    let err = c
        .send_json(raw(HttpMethod::Get, &format!("{}/steal", mock.base), None))
        .await
        .err();
    assert!(
        matches!(err, Some(Error::Input(InputError::ForeignOrigin { .. }))),
        "{err:?}"
    );
    assert_eq!(mock.hits(), 0);
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Dropped {
    OutcomeUnknown,
    Network,
    Other,
}

impl Dropped {
    fn of(err: Option<&Error>) -> Self {
        match err {
            Some(Error::Http(HttpError::NoResponse { .. })) => Self::OutcomeUnknown,
            Some(Error::Http(HttpError::Transport(_))) => Self::Network,
            _ => Self::Other,
        }
    }
}

#[rstest]
#[case::write(HttpMethod::Post, Dropped::OutcomeUnknown)]
#[case::read(HttpMethod::Get, Dropped::Network)]
#[tokio::test]
async fn dropped_connection_after_send(
    #[case] method: HttpMethod,
    #[case] expected: Dropped,
) -> TestResult {
    let mock = common::script(vec![(common::DROP, "", "")]).await?;
    let err = mock
        .client(TOKEN)?
        .send_json(raw(method, "/gapi/channel/create", None))
        .await
        .err();
    assert_eq!(Dropped::of(err.as_ref()), expected, "{err:?}");
    assert_eq!(mock.hits(), 1, "a failed exchange is not repeated");
    Ok(())
}

// The path of a POST whose outcome is unknown.
fn unknown_post_path(err: Option<&Error>) -> Option<&str> {
    match err {
        Some(Error::Http(HttpError::NoResponse {
            method: "POST",
            path,
        })) => Some(path),
        _ => None,
    }
}

#[tokio::test]
async fn timed_out_write_has_unknown_outcome() -> TestResult {
    let mock = common::scripted(Duration::from_secs(2), vec![(200, "", "{}")]).await?;
    let client = mock.limited_client(Duration::from_millis(200), 1024)?;
    let err = client
        .send_json(raw(HttpMethod::Post, "/gapi/channel/create", None))
        .await
        .err();
    assert_eq!(
        unknown_post_path(err.as_ref()),
        Some("/gapi/channel/create"),
        "{err:?}"
    );
    Ok(())
}

#[rstest]
#[case::declared_length("")]
#[case::close_delimited(common::CLOSE_DELIMITED)]
#[tokio::test]
async fn oversized_body_is_refused(#[case] headers: &'static str) -> TestResult {
    let mock =
        common::script(vec![(200, headers, r#"{"padding":"0123456789abcdef"}"#)])
            .await?;
    let client = mock.limited_client(Duration::from_secs(5), 8)?;
    let err = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await
        .err();
    assert!(
        matches!(
            err,
            Some(Error::Http(HttpError::ResponseTooLarge { limit: 8 }))
        ),
        "{err:?}"
    );
    Ok(())
}

#[tokio::test]
async fn close_delimited_body_under_the_limit_is_read() -> TestResult {
    let mock =
        common::script(vec![(200, common::CLOSE_DELIMITED, r#"{"ok":true}"#)]).await?;
    let client = mock.limited_client(Duration::from_secs(5), 1024)?;
    let value = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;
    assert_eq!(value.get("ok").and_then(Value::as_bool), Some(true));
    Ok(())
}

#[tokio::test]
async fn bare_token_is_explained_and_stops() -> TestResult {
    let mock = common::script(vec![(401, "", "")]).await?;
    let client = mock.client_with(TOKEN, "")?;
    let first = client
        .send_json(raw(HttpMethod::Get, "/gapi/current_user", None))
        .await
        .err();
    assert!(
        matches!(first, Some(Error::Login(LoginError::BareToken { .. }))),
        "{first:?}"
    );
    let second = client
        .send_json(raw(HttpMethod::Get, "/gapi/current_user", None))
        .await
        .err();
    assert!(
        matches!(second, Some(Error::Login(LoginError::SessionRevoked))),
        "{second:?}"
    );
    assert_eq!(mock.hits(), 1);
    Ok(())
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
#[tokio::test]
async fn dot_segments_are_refused_before_request(#[case] id: &str) -> TestResult {
    let mock = common::script(vec![(200, "", "{}")]).await?;
    let err =
        holst_core::remote::api::channel::delete_request(id, DeleteMode::Trash).err();
    assert!(
        matches!(err, Some(Error::Input(InputError::NotPathSegment { .. }))),
        "{err:?}"
    );
    assert_eq!(mock.hits(), 0);
    Ok(())
}

#[tokio::test]
async fn server_can_drop_a_cookie() -> TestResult {
    let mock = common::script(vec![
        (200, "set-cookie: a=1\r\nset-cookie: b=2\r\n", "{}"),
        (200, "set-cookie: a=; Max-Age=0\r\n", "{}"),
        (200, "", r#"{"seen":"{cookie}"}"#),
    ])
    .await?;
    let client = mock.client(TOKEN)?;
    client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;
    client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;
    let value = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;

    let expected = format!("b=2; {}", common::SESSION_COOKIE);
    assert_eq!(
        value.get("seen").and_then(Value::as_str),
        Some(expected.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn download_carries_session_cookie() -> TestResult {
    let mock = common::script(vec![
        (200, "set-cookie: sessionToken=abc\r\n", "{}"),
        (200, "", "{cookie}"),
    ])
    .await?;
    let client = mock.client(TOKEN)?;
    client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;
    let bytes = client.download("/gapi/backup/download").await?;

    assert_eq!(
        String::from_utf8_lossy(&bytes),
        format!("sessionToken=abc; {}", common::SESSION_COOKIE)
    );
    Ok(())
}
