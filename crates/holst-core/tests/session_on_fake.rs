//! Logging in, checking and ending a login against the stateful fake server.

use holst_core::{
    command::{
        account::{SessionCheck, check},
        login::{ServerSessionEnd, email::EmailLogin, logout},
    },
    domain::session::{Session, SessionStatus},
    fake::{Fake, Fixture, LOGIN_CODE},
    remote::client::Client,
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;

fn connect(
    session: Option<Session>,
    base: holst_core::domain::url::BaseUrl,
) -> holst_core::Result<Client> {
    holst_core::command::login::connect(session, base, SystemClock, RandomIds)
}

async fn signed_in(fake: &Fake) -> holst_core::Result<Client> {
    let anonymous = connect(None, fake.base_url().parse()?)?;
    let sent = EmailLogin::new(anonymous, "you@example.com".parse()?)
        .send_code()
        .await?;
    sent.log_in(&SecretString::from(LOGIN_CODE)).await
}

fn rejection(verdict: SessionCheck) -> Option<String> {
    match verdict {
        SessionCheck::Revoked(err) => Some(err.to_string()),
        SessionCheck::Active(_) | SessionCheck::Unknown(_) => None,
    }
}

#[tokio::test]
async fn a_wrong_code_gives_no_login() -> holst_core::Result<()> {
    let fake = Fake::start(Fixture::default()).await?;
    let anonymous = connect(None, fake.base_url().parse()?)?;
    let sent = EmailLogin::new(anonymous, "you@example.com".parse()?)
        .send_code()
        .await?;
    let refused = sent.log_in(&SecretString::from("000000")).await;
    assert_eq!(
        refused.err().as_ref().and_then(holst_core::Error::status),
        Some(400)
    );
    Ok(())
}

#[tokio::test]
async fn a_login_the_server_does_not_know_is_rejected_in_words()
-> holst_core::Result<()> {
    let fake = Fake::start(Fixture::default()).await?;
    let session = Session::with_cookies(
        SecretString::from("csrf-x"),
        "sessionToken=unknown".parse().unwrap_or_default(),
    );
    let stranger = connect(Some(session), fake.base_url().parse()?)?;
    assert_eq!(
        rejection(check(&stranger).await).as_deref(),
        Some("the saved login was revoked or has expired")
    );
    assert_eq!(stranger.status(), SessionStatus::Revoked);
    Ok(())
}

#[tokio::test]
async fn logging_out_ends_the_session_and_forgets_it() -> holst_core::Result<()> {
    let fake = Fake::start(Fixture::default()).await?;
    let client = signed_in(&fake).await?;
    let session = client.session();
    assert_eq!(logout(&client).await?, ServerSessionEnd::Ended);
    assert_eq!(client.status(), SessionStatus::Missing);
    let again = connect(session, fake.base_url().parse()?)?;
    assert!(matches!(check(&again).await, SessionCheck::Revoked(_)));
    assert_eq!(logout(&again).await?, ServerSessionEnd::AlreadyEnded);
    Ok(())
}
