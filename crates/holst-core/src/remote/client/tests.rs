#![expect(
    clippy::panic_in_result_fn,
    reason = "asserting is the point of a test"
)]

use std::{sync::Arc, time::Duration};

use rstest::rstest;
use serde_json::Value;

use crate::{
    domain::{
        session::{Session, SessionStatus},
        url::DEFAULT_BASE,
    },
    effect::{
        clock::FixedClock,
        http::{HttpBody, HttpMethod, HttpRequest},
        request::{ApiRequest, Authorization, QueryParameter},
    },
    error::{
        Error,
        Result,
        hint::Hint,
        http::HttpError,
        local_data::LocalDataError,
        login::LoginError,
    },
    remote::client::{
        Client,
        MAX_REDIRECTS,
        fake::{Fake, connect, cookie_line, lost, new_client, on_fake, reply, stored},
    },
};

fn bare_rejection_path(err: Option<&Error>) -> Option<&str> {
    match err {
        Some(Error::Login(LoginError::BareToken { path, .. })) => Some(path.as_str()),
        Some(_) | None => None,
    }
}

fn rate_limit_pause(err: Option<&Error>) -> Option<u64> {
    match err {
        Some(Error::Http(HttpError::RateLimited { retry_after, .. })) => {
            Some(retry_after.as_secs())
        }
        Some(_) | None => None,
    }
}

fn refused_location(err: Option<&Error>) -> Option<&str> {
    match err {
        Some(Error::Http(HttpError::ForeignRedirect { location })) => {
            Some(location.as_str())
        }
        Some(_) | None => None,
    }
}

fn html_status(err: Option<&Error>) -> Option<u16> {
    match err {
        Some(Error::Http(HttpError::HtmlInsteadOfJson { status })) => Some(*status),
        Some(_) | None => None,
    }
}

fn anonymous(path: &str) -> ApiRequest {
    ApiRequest {
        method: HttpMethod::Post,
        path: path.to_owned(),
        query: Vec::new(),
        body: None,
        authorization: Authorization::Anonymous,
    }
}

fn client(token: &str) -> Result<Client> {
    new_client(Some(Session::new(token.into())), DEFAULT_BASE.parse()?)
}

fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request.headers.get(name).and_then(|v| v.to_str().ok())
}

fn paths(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

fn bare(token: &str) -> Session {
    Session::new(token.into())
}

#[test]
fn stored_cookies_come_back_sorted() -> Result<()> {
    let client = new_client(Some(stored("tok", "b=2; a=1")), DEFAULT_BASE.parse()?)?;
    assert_eq!(client.cookie_count(), 2);
    assert_eq!(
        client.session().map(|s| cookie_line(&s)).as_deref(),
        Some("a=1; b=2")
    );
    Ok(())
}

#[test]
fn debug_does_not_print_token() -> Result<()> {
    let shown = format!("{:?}", client("secret-token-123")?);
    assert!(!shown.contains("secret-token-123"), "{shown}");
    assert!(shown.contains("Active"), "{shown}");
    Ok(())
}

#[tokio::test]
async fn cookies_from_replies_ride_along() -> Result<()> {
    let (fake, client) = on_fake(
        [
            reply(
                200,
                &[
                    ("set-cookie", "s=1; Path=/"),
                    ("set-cookie", "old=; Max-Age=0"),
                ],
                "{}",
            ),
            reply(200, &[], r#"{"id":"u1"}"#),
        ],
        Some(stored("tok", "old=x")),
    )?;
    client
        .send_json(anonymous("/gapi/login/with_email"))
        .await?;
    let me = client
        .send_json(raw(HttpMethod::Get, "/gapi/current_user", None))
        .await?;
    assert_eq!(me.get("id"), Some(&Value::from("u1")));
    let seen = fake.requests();
    let cookies: Vec<_> = seen.iter().map(|r| header(r, "cookie")).collect();
    assert_eq!(cookies, [Some("old=x"), Some("s=1")]);
    let authorization: Vec<_> =
        seen.iter().map(|r| header(r, "authorization")).collect();
    assert_eq!(authorization, [None, Some("Bearer tok")]);
    assert_eq!(
        client.session().map(|s| cookie_line(&s)).as_deref(),
        Some("s=1")
    );
    Ok(())
}

#[tokio::test]
async fn answered_counts_the_successes_of_every_clone() -> Result<()> {
    let (_fake, client) = on_fake(
        [reply(200, &[], "{}"), reply(401, &[], "")],
        Some(stored("tok", "sid=1")),
    )?;
    let clone = client.clone();
    let request = || raw(HttpMethod::Get, "/gapi/current_user", None);
    client.send_json(request()).await?;
    clone.send_json(request()).await.err();
    assert_eq!(client.answered(), 1);
    Ok(())
}

#[rstest]
#[case::expired(stored("tok", "sid=1"), &[], Hint::LoginAgain)]
#[case::cookie_cleared_in_reply(stored("tok", "sid=1"), &[("set-cookie", "sid=; Max-Age=0")], Hint::LoginAgain)]
#[case::bare_token(Session::new("tok".into()), &[], Hint::FullLogin)]
#[tokio::test]
async fn unauthorized_revokes_the_session_for_all_clones(
    #[case] session: Session,
    #[case] headers: &[(&'static str, &str)],
    #[case] hint: Hint,
) -> Result<()> {
    let (fake, client) = on_fake([reply(401, headers, "")], Some(session))?;
    let clone = client.clone();
    let first = client
        .send_json(raw(HttpMethod::Get, "/gapi/current_user", None))
        .await
        .err();
    let bare = bare_rejection_path(first.as_ref()) == Some("/gapi/current_user");
    let revoked = matches!(first, Some(Error::Login(LoginError::SessionRevoked)));
    assert!(bare || revoked, "{first:?}");
    assert!(first.as_ref().is_some_and(Error::is_fatal), "{first:?}");
    assert_eq!(first.and_then(|e| e.hint()), Some(hint));
    assert_eq!(clone.status(), SessionStatus::Revoked);
    let second = clone
        .send_json(raw(HttpMethod::Get, "/gapi/current_user", None))
        .await;
    assert!(matches!(
        second,
        Err(Error::Login(LoginError::SessionRevoked))
    ));
    assert_eq!(
        fake.requests().len(),
        1,
        "a revoked session went to the network"
    );
    assert!(client.session().is_none());
    Ok(())
}

#[tokio::test]
async fn anonymous_401_does_not_revoke() -> Result<()> {
    let (_, client) = on_fake([reply(401, &[], "")], None)?;
    let err = client
        .send_json(anonymous("/gapi/login/with_email"))
        .await
        .err();
    assert!(
        matches!(
            err,
            Some(Error::Http(HttpError::Status { status: 401, .. }))
        ),
        "{err:?}"
    );
    assert_eq!(client.status(), SessionStatus::Missing);
    Ok(())
}

#[test]
fn forget_drops_login_and_cookies_on_every_clone() -> Result<()> {
    let client = new_client(Some(stored("tok", "sid=1")), DEFAULT_BASE.parse()?)?;
    let clone = client.clone();
    client.forget();
    assert_eq!(
        (clone.status(), clone.cookie_count()),
        (SessionStatus::Missing, 0)
    );
    Ok(())
}

#[tokio::test]
async fn echoed_cookie_values_are_scrubbed() -> Result<()> {
    let (_, client) = on_fake(
        [reply(
            400,
            &[],
            r#"{"cookie":"sid=s3ss10n-v4lue; theme=dark","auth":"Bearer tok-42"}"#,
        )],
        Some(stored("tok-42", "sid=s3ss10n-v4lue; theme=dark")),
    )?;
    let err = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await
        .err();
    assert_eq!(
        err.map(|e| e.to_string()).as_deref(),
        Some(
            r#"GET /gapi/x -> HTTP 400: {"cookie":"sid=***; theme=dark","auth":"Bearer ***"}"#
        )
    );
    Ok(())
}

#[tokio::test]
async fn rotated_cookies_are_offered_until_confirmed() -> Result<()> {
    let (_, client) = on_fake(
        [
            reply(200, &[("set-cookie", "sid=1")], "{}"),
            reply(200, &[("set-cookie", "sid=2")], "{}"),
        ],
        Some(stored("tok", "sid=1")),
    )?;
    client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;
    assert!(
        client.rotated().is_none(),
        "the same cookie is not a rotation"
    );
    client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await?;
    let rotated = client.rotated();
    assert_eq!(rotated.as_ref().map(cookie_line).as_deref(), Some("sid=2"));
    assert!(
        client.rotated().is_some(),
        "looking does not clear the mark"
    );
    client.confirm_saved(&rotated.ok_or(Error::Login(LoginError::NoSavedLogin))?);
    assert!(client.rotated().is_none(), "a confirmed save clears it");
    Ok(())
}

#[cfg(feature = "logging")]
async fn logged(work: impl Future<Output = Result<Value>>) -> (Result<Value>, String) {
    use std::sync::{Mutex, PoisonError};

    use tracing::instrument::WithSubscriber as _;

    struct Lines(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&buffer);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
        .with_writer(move || Lines(Arc::clone(&shared)))
        .finish();
    let outcome = work.with_subscriber(subscriber).await;
    let text =
        String::from_utf8_lossy(&buffer.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned();
    (outcome, text)
}

fn raw(method: HttpMethod, path: &str, body: Option<&Value>) -> ApiRequest {
    ApiRequest {
        method,
        path: path.to_owned(),
        query: Vec::new(),
        body: body.and_then(|value| HttpBody::json(value).ok()),
        authorization: Authorization::Session,
    }
}

#[cfg(feature = "logging")]
#[tokio::test]
async fn request_logs_never_carry_secrets() -> Result<()> {
    let (_, client) = on_fake(
        [reply(200, &[("set-cookie", "sid=fresh-SECRET-3")], "{}")],
        Some(stored("tok-SECRET-1", "sid=old-SECRET-2")),
    )?;
    let request = ApiRequest {
        query: vec![QueryParameter {
            name: "key".to_owned(),
            value: "SECRET-4".to_owned(),
        }],
        ..raw(HttpMethod::Get, "/gapi/x", None)
    };
    let (outcome, text) = logged(async { client.send_json(request).await }).await;
    outcome?;
    // Positive control: the request line is there, so the absence counts.
    assert!(
        text.contains("request") && text.contains("path=/gapi/x"),
        "{text}"
    );
    assert!(!text.contains("SECRET"), "{text}");
    Ok(())
}

#[tokio::test]
async fn reads_are_retried_and_errors_are_scrubbed() -> Result<()> {
    let (fake, client) = on_fake(
        [
            reply(503, &[("retry-after", "0")], ""),
            reply(403, &[], r#"{"echo":"tok-42"}"#),
        ],
        Some(bare("tok-42")),
    )?;
    let err = client
        .send_json(ApiRequest {
            query: vec![QueryParameter {
                name: "q".to_owned(),
                value: "a b".to_owned(),
            }],
            ..raw(HttpMethod::Get, "/gapi/x", None)
        })
        .await
        .err();
    assert_eq!(fake.requests().len(), 2);
    assert_eq!(
        err.map(|e| e.to_string()).as_deref(),
        Some(r#"GET /gapi/x -> HTTP 403: {"echo":"***"}"#)
    );
    assert_eq!(
        fake.requests()
            .first()
            .map(|r| r.url.to_string())
            .as_deref(),
        Some("https://app.holst.so/gapi/x?q=a+b")
    );
    Ok(())
}

#[rstest]
#[case::get_gives_up_after_three(HttpMethod::Get, 3)]
#[case::post_once(HttpMethod::Post, 1)]
#[case::patch_once(HttpMethod::Patch, 1)]
#[case::delete_once(HttpMethod::Delete, 1)]
#[tokio::test]
async fn gateway_errors_are_retried_for_reads_only(
    #[case] method: HttpMethod,
    #[case] sent: usize,
) -> Result<()> {
    let (fake, client) = on_fake(
        [502, 503, 504].map(|status| reply(status, &[("retry-after", "0")], "")),
        Some(bare("t")),
    )?;
    let err = client.send_json(raw(method, "/gapi/x", None)).await.err();
    assert!(err.as_ref().and_then(Error::status).is_some(), "{err:?}");
    assert_eq!(fake.requests().len(), sent);
    Ok(())
}

#[tokio::test]
async fn lost_write_is_not_retried() -> Result<()> {
    let (fake, client) = connect(
        Fake::scripted([lost("/gapi/channel/create")]),
        Some(bare("t")),
    )?;
    let err = client
        .send_json(raw(HttpMethod::Post, "/gapi/channel/create", None))
        .await
        .err();
    assert!(
        matches!(err, Some(Error::Http(HttpError::NoResponse { .. }))),
        "{err:?}"
    );
    assert_eq!(err.and_then(|e| e.hint()), Some(Hint::CheckBeforeRetry));
    assert_eq!(fake.requests().len(), 1);
    Ok(())
}

#[tokio::test]
async fn long_rate_limit_names_the_pause() -> Result<()> {
    let (fake, client) = on_fake(
        [
            reply(429, &[("retry-after", "0")], ""),
            reply(429, &[("retry-after", "120")], ""),
        ],
        Some(bare("t")),
    )?;
    let err = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await
        .err();
    assert_eq!(rate_limit_pause(err.as_ref()), Some(120), "{err:?}");
    assert!(err.is_some_and(|e| e.to_string().contains("120 s")));
    assert_eq!(fake.requests().len(), 2);
    Ok(())
}

#[tokio::test]
async fn same_origin_redirects_are_followed() -> Result<()> {
    let (fake, client) = on_fake(
        [
            reply(307, &[("location", "/gapi/moved")], ""),
            reply(303, &[("location", "https://app.holst.so/gapi/result")], ""),
            reply(200, &[], r#"{"ok":true}"#),
        ],
        Some(stored("tok", "sid=1")),
    )?;
    let value = client
        .send_json(raw(
            HttpMethod::Post,
            "/gapi/x",
            Some(&serde_json::json!({ "a": 1 })),
        ))
        .await?;
    assert_eq!(value.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(
        paths(&fake),
        ["POST /gapi/x", "POST /gapi/moved", "GET /gapi/result"]
    );
    let seen = fake.requests();
    let bodies: Vec<_> = seen.iter().map(|r| r.body.is_some()).collect();
    assert_eq!(bodies, [true, true, false]);
    assert!(seen.iter().all(|r| header(r, "cookie") == Some("sid=1")));
    Ok(())
}

#[tokio::test]
async fn foreign_redirect_is_refused() -> Result<()> {
    let (fake, client) = on_fake(
        [reply(
            302,
            &[("location", "https://evil.test/steal?x=1")],
            "",
        )],
        Some(stored("tok", "sid=1")),
    )?;
    let err = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await
        .err();
    assert_eq!(
        refused_location(err.as_ref()),
        Some("https://evil.test"),
        "{err:?}"
    );
    assert_eq!(paths(&fake), ["GET /gapi/x"]);
    Ok(())
}

#[tokio::test]
async fn redirect_loop_stops() -> Result<()> {
    let (fake, client) = on_fake(
        (0..10).map(|_| reply(302, &[("location", "/gapi/x")], "")),
        Some(bare("t")),
    )?;
    let err = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await
        .err();
    assert_eq!(err.as_ref().and_then(Error::status), Some(302), "{err:?}");
    assert_eq!(
        fake.requests().len(),
        usize::try_from(MAX_REDIRECTS + 1).unwrap_or(0)
    );
    Ok(())
}

#[rstest]
#[case::success_page(200, "text/html", Some(200))]
#[case::sniffed(200, "", Some(200))]
#[case::error_page(403, "text/html", None)]
#[tokio::test]
async fn html_pages_are_named(
    #[case] status: u16,
    #[case] content_type: &str,
    #[case] html_error: Option<u16>,
) -> Result<()> {
    let headers: Vec<_> = Some(("content-type", content_type))
        .filter(|(_, kind)| !kind.is_empty())
        .into_iter()
        .collect();
    let page = "<!DOCTYPE html><title>Maintenance</title>";
    let (_, client) =
        on_fake([reply(status, &headers, page)], Some(stored("t", "sid=1")))?;
    let err = client
        .send_json(raw(HttpMethod::Get, "/gapi/x", None))
        .await
        .err();
    assert_eq!(html_status(err.as_ref()), html_error, "{err:?}");
    let shown = err.map(|e| e.to_string()).unwrap_or_default();
    assert!(!shown.contains("Maintenance"), "{shown}");
    Ok(())
}

#[tokio::test]
async fn retry_waits_on_the_client_clock() -> Result<()> {
    let (fake, client) = on_fake(
        [
            reply(429, &[("retry-after", "30")], ""),
            reply(200, &[], r#"{"ok":true}"#),
        ],
        Some(bare("t")),
    )?;
    let client = client.with_clock(FixedClock { unix_millis: 0 });
    let value = tokio::time::timeout(
        Duration::from_secs(5),
        client.send_json(raw(HttpMethod::Get, "/gapi/x", None)),
    )
    .await
    .map_err(|_| Error::LocalData(LocalDataError::KeychainLocked))??;
    assert_eq!(value.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(fake.requests().len(), 2);
    Ok(())
}

#[tokio::test]
async fn download_link_is_unsigned_retried_and_settled() -> Result<()> {
    let link = "https://app.holst.so/hud/updates/1";
    let (fake, client) = on_fake(
        [
            reply(503, &[("retry-after", "0")], ""),
            reply(200, &[], "bytes"),
            reply(404, &[], "gone"),
        ],
        Some(stored("tok", "sid=1")),
    )?;
    assert_eq!(client.download_trusted_link(link).await?, b"bytes");
    let err = client.download_trusted_link(link).await.err();
    assert_eq!(
        err.map(|e| e.to_string()).as_deref(),
        Some("GET /hud/updates/1 -> HTTP 404: gone")
    );
    let seen = fake.requests();
    let signed: Vec<_> = seen
        .iter()
        .map(|r| (header(r, "authorization"), header(r, "cookie")))
        .collect();
    assert_eq!(signed, [(None, None); 3]);
    assert_eq!(client.status(), SessionStatus::Active);
    Ok(())
}

#[tokio::test]
async fn a_limited_client_keeps_its_transport() -> Result<()> {
    let fake = Arc::new(Fake::scripted([Ok(reply(200, &[], "bytes"))]));
    // Loopback, nothing listens: a client off the fake fails locally.
    let client = new_client(Some(bare("tok")), "http://127.0.0.1:1".parse()?)?
        .with_transport(Arc::clone(&fake))
        .with_response_limit(Duration::from_secs(1), 16)?;
    assert_eq!(client.download("/x").await?, b"bytes");
    assert_eq!(paths(&fake), ["GET /x"]);
    Ok(())
}

#[tokio::test]
async fn a_limited_fake_refuses_a_body_over_the_limit() -> Result<()> {
    let fake = Arc::new(Fake::scripted([
        Ok(reply(200, &[], "0123456789")),
        Ok(reply(200, &[], "012345678")),
    ]));
    let client = new_client(Some(bare("tok")), "http://127.0.0.1:1".parse()?)?
        .with_transport(Arc::clone(&fake))
        .with_response_limit(Duration::from_secs(1), 9)?;
    let err = client.download("/x").await.err();
    assert!(
        matches!(
            err,
            Some(Error::Http(HttpError::ResponseTooLarge { limit: 9 }))
        ),
        "{err:?}"
    );
    assert_eq!(client.download("/x").await?.len(), 9);
    Ok(())
}
