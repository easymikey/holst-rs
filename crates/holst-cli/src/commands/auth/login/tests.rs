#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use std::sync::Mutex;

use secrecy::ExposeSecret as _;

fn exposed(secret: &SecretString) -> &str {
    secret.expose_secret()
}

use holst_core::{
    command::{auth::SaveLogin, login::ReplacedLogin},
    domain::{
        login::Email,
        session::{Session, SessionStatus},
        url::BaseUrl,
    },
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::{
    commands::{
        auth::login::{Done, conclude, finish, log_in_by_email, parse_secret},
        fake::FakeServer,
    },
    error::{Error, InvalidAnswer, Result, SecretKind},
    form::{
        Form,
        draw::NoteTone,
        fake::{Script, screen},
        line::Keystroke,
        terminal::TerminalKind,
    },
};

fn client(server: &std::sync::Arc<FakeServer>) -> Result<Client> {
    let base: BaseUrl = "https://app.holst.so".parse()?;
    Ok(holst_core::command::login::connect(
        None,
        base,
        holst_core::system::clock::SystemClock,
        holst_core::system::id::RandomIds,
    )?
    .with_transport(std::sync::Arc::clone(server)))
}

fn you() -> Result<Email> {
    Ok("you@example.com".parse()?)
}

fn chars(text: &'static str) -> impl Iterator<Item = Keystroke> {
    text.chars().map(Keystroke::Char).chain([Keystroke::Enter])
}

const OPTIONS: &str = "/gapi/login/get_public_options";
const SEND: &str = "/gapi/send_temporary_password";
const LOGIN: &str = "/gapi/login/with_email";
const ME: &str = "/gapi/current_user";

fn code_account() -> Value {
    json!({ "hasAccount": true, "viaTemporaryPassword": true })
}

#[tokio::test]
async fn email_login_goes_through_every_step() -> Result<()> {
    let server = FakeServer::responding(&[
        (OPTIONS, 200, code_account()),
        (SEND, 200, json!({})),
        (LOGIN, 200, json!({ "token": "csrf-1" })),
        (ME, 200, json!({ "id": "u1", "name": "Mock User" })),
    ]);
    let strokes = chars("you@example.com").chain(chars("123456"));
    let form =
        Form::intro(Script::new(TerminalKind::Live, strokes, []), "holst login")?;
    let granted = log_in_by_email(&form, client(&server)?, None).await?;
    let (done, _) = conclude(granted, None, SaveLogin::Skip, async |_| {
        Ok(LoginSaveReport {
            store: SessionStoreKind::File,
            stale: None,
        })
    })
    .await?;
    assert_eq!(done.name, "Mock User");
    assert_eq!(
        done.lines(ReplacedLogin::Nothing, None),
        [
            (NoteTone::Plain, "Mock User".to_owned()),
            (NoteTone::Faint, "not saved (--no-save)".to_owned())
        ]
    );
    assert_eq!(
        server.seen(),
        [
            format!("POST {OPTIONS}"),
            format!("POST {SEND}"),
            format!("POST {LOGIN}"),
            format!("GET {ME}")
        ]
    );
    let raw = form.terminal().written();
    assert!(!raw.contains("123456"), "{raw:?}");
    let shown = screen(&raw);
    assert!(shown.contains("◇  Code\n│  ▪▪▪▪▪▪"), "{shown}");
    assert!(!shown.contains("Code sent"), "{shown}");
    assert!(!shown.contains("Sending the code"), "{shown}");
    assert_eq!(shown.matches("you@example.com").count(), 1, "{shown}");
    Ok(())
}

#[tokio::test]
async fn wrong_code_is_asked_again() -> Result<()> {
    let server = FakeServer::responding(&[
        (OPTIONS, 200, code_account()),
        (SEND, 200, json!({})),
        (LOGIN, 400, json!({ "errCode": "bad-request" })),
        (LOGIN, 200, json!({ "token": "csrf-1" })),
    ]);
    let strokes = chars("111111").chain(chars("123456"));
    let form =
        Form::intro(Script::new(TerminalKind::Live, strokes, []), "holst login")?;
    let granted = log_in_by_email(&form, client(&server)?, Some(you()?)).await?;
    assert_eq!(granted.status(), SessionStatus::Active);
    let raw = form.terminal().written();
    assert!(raw.contains("▲ that code did not match"), "{raw:?}");
    Ok(())
}

#[tokio::test]
async fn an_email_padded_with_spaces_is_trimmed() -> Result<()> {
    let server = FakeServer::responding(&[
        (OPTIONS, 200, code_account()),
        (SEND, 200, json!({})),
        (LOGIN, 200, json!({ "token": "csrf-1" })),
    ]);
    let strokes = chars("  you@example.com  ").chain(chars("123456"));
    let form =
        Form::intro(Script::new(TerminalKind::Live, strokes, []), "holst login")?;
    let granted = log_in_by_email(&form, client(&server)?, None).await?;
    assert_eq!(granted.status(), SessionStatus::Active);
    let raw = form.terminal().written();
    assert!(!raw.contains("enter an address"), "{raw:?}");
    Ok(())
}

#[tokio::test]
async fn piped_wrong_code_is_an_error() -> Result<()> {
    let server = FakeServer::responding(&[
        (OPTIONS, 200, code_account()),
        (SEND, 200, json!({})),
        (LOGIN, 400, json!({})),
    ]);
    let form = Form::intro(
        Script::new(TerminalKind::Redirected, [], ["000000"]),
        "holst login",
    )?;
    let err = log_in_by_email(&form, client(&server)?, Some(you()?))
        .await
        .err();
    assert!(
        matches!(
            err,
            Some(Error::WrongSecret {
                secret: SecretKind::Code,
                ..
            })
        ),
        "{err:?}"
    );
    Ok(())
}

#[rstest]
#[case::no_account(json!({}), "there is no account")]
#[case::sso_only(json!({ "hasAccount": true }), "only log in through SSO")]
#[tokio::test]
async fn no_way_in_stops_before_any_code(
    #[case] offered: Value,
    #[case] message: &str,
) -> Result<()> {
    let server = FakeServer::responding(&[(OPTIONS, 200, offered)]);
    let form =
        Form::intro(Script::new(TerminalKind::Redirected, [], []), "holst login")?;
    let err = log_in_by_email(&form, client(&server)?, Some(you()?))
        .await
        .err();
    assert!(err.is_some_and(|err| err.to_string().contains(message)));
    assert_eq!(server.seen(), [format!("POST {OPTIONS}")]);
    Ok(())
}

#[tokio::test]
async fn refused_session_reads_as_a_rejected_login() -> Result<()> {
    let server = FakeServer::responding(&[(ME, 401, json!({}))]);
    let granted = client(&server)?.with_token(SecretString::from("t"));
    let err = conclude(granted, None, SaveLogin::Skip, async |_| {
        Ok(LoginSaveReport {
            store: SessionStoreKind::File,
            stale: None,
        })
    })
    .await
    .err();
    assert!(matches!(err, Some(Error::SessionNotAccepted(_))), "{err:?}");
    Ok(())
}

#[tokio::test]
async fn saved_session_goes_to_the_store_with_its_account() -> Result<()> {
    let id = "aaaaaaaa-0000-4000-8000-000000000001";
    let server = FakeServer::responding(&[(ME, 200, json!({ "id": id, "name": "A" }))]);
    let granted = client(&server)?.with_token(SecretString::from("tok"));
    let kept = Mutex::new(None);
    let (done, _) = conclude(granted, None, SaveLogin::Keep, async |session| {
        *kept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((
            exposed(session.token()).to_owned(),
            session.account_id().map(str::to_owned),
        ));
        Ok(LoginSaveReport {
            store: SessionStoreKind::Keychain,
            stale: None,
        })
    })
    .await?;
    assert_eq!(done.store, Some(SessionStoreKind::Keychain));
    assert_eq!(
        kept.lock().ok().and_then(|k| k.clone()),
        Some((String::from("tok"), Some(id.to_owned())))
    );
    Ok(())
}

#[test]
fn login_closes_with_a_boxed_note() -> Result<()> {
    let form =
        Form::intro(Script::new(TerminalKind::Redirected, [], []), "holst login")?;
    let done = Done {
        name: "A".into(),
        store: Some(SessionStoreKind::Keychain),
        stale: None,
        cookies: 2,
    };
    finish(&form, &done, ReplacedLogin::Terminated, Some("Keychain"))?;
    let shown = screen(&form.terminal().written());
    assert!(shown.contains("◇  Logged in ─"), "{shown}");
    assert!(
        shown.contains("│  saved to Keychain · 2 cookies"),
        "{shown}"
    );
    assert!(shown.contains("│  previous session terminated"), "{shown}");
    assert!(shown.ends_with("╯\n│\n└  Done"), "{shown}");
    Ok(())
}

fn old_client(server: &std::sync::Arc<FakeServer>) -> Result<Client> {
    let base: BaseUrl = "https://app.holst.so".parse()?;
    let jar: holst_core::domain::cookie::CookieJar =
        "sessionToken=old".parse().unwrap_or_default();
    let session = Session::with_cookies(SecretString::from("csrf-old"), jar);
    Ok(holst_core::command::login::connect(
        Some(session),
        base,
        holst_core::system::clock::SystemClock,
        holst_core::system::id::RandomIds,
    )?
    .with_transport(std::sync::Arc::clone(server)))
}

#[rstest]
#[case::terminated(200, ReplacedLogin::Terminated)]
#[case::already_gone(401, ReplacedLogin::Terminated)]
#[case::server_error(500, ReplacedLogin::Kept)]
#[tokio::test]
async fn previous_session_is_terminated_with_its_own_cookie(
    #[case] status: u16,
    #[case] expected: ReplacedLogin,
) -> Result<()> {
    let server = FakeServer::responding(&[
        (ME, 200, json!({ "id": "u1", "name": "A" })),
        ("/gapi/logout", status, json!({})),
    ]);
    let granted = client(&server)?.with_token(SecretString::from("tok"));
    let (_, replaced) = conclude(
        granted,
        Some(old_client(&server)?),
        SaveLogin::Keep,
        async |_| {
            Ok(LoginSaveReport {
                store: SessionStoreKind::File,
                stale: None,
            })
        },
    )
    .await?;
    assert_eq!(replaced, expected);
    assert_eq!(
        server.seen(),
        [format!("GET {ME}"), "POST /gapi/logout".to_owned()]
    );
    assert!(
        server
            .cookies()
            .last()
            .is_some_and(|cookie| cookie.contains("sessionToken=old"))
    );
    Ok(())
}

#[test]
fn closing_lines_name_the_store_and_the_previous_session() {
    let done = Done {
        name: "A".into(),
        store: Some(SessionStoreKind::File),
        stale: None,
        cookies: 1,
    };
    let lines = done.lines(ReplacedLogin::Kept, Some("/tmp/h/session"));
    assert_eq!(
        lines,
        [
            (NoteTone::Plain, "A".to_owned()),
            (
                NoteTone::Faint,
                "saved to /tmp/h/session · 1 cookie".to_owned()
            ),
            (
                NoteTone::Warn,
                "could not terminate the previous session".to_owned()
            ),
        ]
    );
}

#[test]
fn closing_lines_warn_about_an_older_login() {
    let done = Done {
        name: "A".into(),
        store: Some(SessionStoreKind::File),
        stale: Some("an older login may be left".into()),
        cookies: 1,
    };
    let lines = done.lines(ReplacedLogin::Nothing, Some("/h/token"));
    assert_eq!(
        lines.get(2),
        Some(&(NoteTone::Warn, "an older login may be left".to_owned()))
    );
}

#[test]
fn empty_secret_is_not_an_answer() {
    assert_eq!(parse_secret("").err(), Some(InvalidAnswer::Empty));
    assert_eq!(
        parse_secret("919426").ok().as_ref().map(exposed),
        Some("919426")
    );
}
