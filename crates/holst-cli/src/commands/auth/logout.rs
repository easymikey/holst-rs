//! `holst auth logout`: end the server session and forget the saved login.

use std::fmt;

use holst_core::{
    command::{
        account,
        auth::{LogoutArgs, LogoutMode},
        login::{self, ServerSessionEnd},
    },
    domain::text::{joined, printable},
    remote::client::Client,
    storage::session_store::SessionStoreKind,
};
use serde::Serialize;

use crate::{
    error::Result,
    output,
    output::OutputFormat,
    run::{
        RunSettings,
        local_data::{forget_cache, forgotten, leftovers_note},
    },
};

pub(crate) async fn logout(
    logout: LogoutArgs,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let saved = saved_client(run).await;
    let account = whose_cache(
        logout.local_only,
        saved.as_ref().ok().and_then(Option::as_ref),
    )
    .await;
    let server = end_session(logout.local_only, saved).await;
    // Forget before printing: a closed stdout must not leave the session on disk.
    let cleared = run.clear().await?;
    forget_cache(run, account.as_deref());
    let (gone, kept) = forgotten(cleared);
    let logged_out = LogoutReport {
        server,
        forgotten: gone.into_iter().map(SessionStoreKind::as_str).collect(),
        kept: kept.is_some(),
    };
    output::report(format, &logged_out)?;
    if let Some(text) = leftovers_note(run) {
        output::stderr_line(text);
    }
    kept.map_or(Ok(()), Err)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LogoutReport {
    server: ServerSessionReport,
    forgotten: Vec<&'static str>,
    #[serde(skip)]
    kept: bool,
}

impl fmt::Display for LogoutReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.server
            .line()
            .map_or(Ok(()), |line| writeln!(f, "{line}"))?;
        match (self.forgotten.as_slice(), self.kept) {
            ([], false) => write!(f, "there was no saved login"),
            ([], true) => write!(f, "the saved login could not be removed"),
            (gone, _) => write!(f, "forgot the login in: {}", joined(gone, ", ")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ServerSessionReport {
    Skipped,
    Unread,
    Ended,
    AlreadyEnded,
    NotEnded,
}

impl ServerSessionReport {
    const fn line(self) -> Option<&'static str> {
        match self {
            ServerSessionReport::Ended => Some("ended the session on the server"),
            ServerSessionReport::AlreadyEnded => {
                Some("the session had already ended on the server")
            }
            ServerSessionReport::Skipped
            | ServerSessionReport::Unread
            | ServerSessionReport::NotEnded => None,
        }
    }
}

async fn whose_cache(scope: LogoutMode, saved: Option<&Client>) -> Option<String> {
    match (scope, saved) {
        (_, None) => None,
        (LogoutMode::LocalOnly, Some(client)) => client.account_id(),
        (LogoutMode::Everywhere, Some(client)) => {
            account::known_account_id(client).await
        }
    }
}

async fn saved_client(run: &RunSettings) -> Result<Option<Client>> {
    run.load().await.and_then(|loaded| {
        loaded
            .map(|(session, _)| run.client(Some(session)))
            .transpose()
    })
}

async fn end_session(
    scope: LogoutMode,
    saved: Result<Option<Client>>,
) -> ServerSessionReport {
    let loaded = match scope {
        LogoutMode::Everywhere => saved,
        LogoutMode::LocalOnly => Ok(None),
    };
    match loaded {
        Ok(Some(client)) => match login::logout(&client).await {
            Ok(ServerSessionEnd::Ended) => ServerSessionReport::Ended,
            Ok(ServerSessionEnd::AlreadyEnded) => ServerSessionReport::AlreadyEnded,
            Err(err) => {
                output::warn(printable(&format!(
                    "could not end the session on the server: {err}"
                )));
                ServerSessionReport::NotEnded
            }
        },
        Ok(None) => ServerSessionReport::Skipped,
        Err(err) => {
            output::warn_with(
                "the session on the server was not ended: the saved login could not be read",
                &err,
            );
            ServerSessionReport::Unread
        }
    }
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::auth::LogoutMode,
        storage::session_store::{LoginRemoval, SessionStoreKind},
    };
    use serde_json::json;

    use crate::{
        commands::auth::logout::{LogoutReport, ServerSessionReport, whose_cache},
        error::Result,
        run::local_data::forgotten,
    };

    #[test]
    fn logout_reports_a_store_that_kept_the_login() {
        let (gone, kept) = forgotten(vec![
            (
                SessionStoreKind::Keychain,
                Err(holst_core::Error::LocalData(
                    holst_core::error::local_data::LocalDataError::KeychainLocked,
                )),
            ),
            (SessionStoreKind::File, Ok(LoginRemoval::Removed)),
        ]);
        assert_eq!(gone, [SessionStoreKind::File]);
        let kept = kept.map(|err| (err.to_string(), err.hint().map(|h| h.to_string())));
        assert_eq!(
            kept,
            Some((
                format!(
                    "could not remove the saved login from the {}",
                    SessionStoreKind::Keychain.as_str()
                ),
                Some(
                    "unlock the keychain and allow access, or keep the login in `HOLST_HOME`"
                        .to_owned()
                )
            ))
        );
        let (absent_gone, absent_kept) =
            forgotten(vec![(SessionStoreKind::Keychain, Ok(LoginRemoval::Absent))]);
        assert!(absent_gone.is_empty() && absent_kept.is_none());
    }

    #[tokio::test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    async fn logout_knows_whose_cache_goes() -> Result<()> {
        use holst_core::domain::session::Session;
        use secrecy::SecretString;

        use crate::commands::fake::FakeServer;

        let id = "aaaaaaaa-0000-4000-8000-000000000001";
        let server =
            FakeServer::responding(&[("/gapi/current_user", 200, json!({ "id": id }))]);
        let base: holst_core::domain::url::BaseUrl = "https://app.holst.so".parse()?;
        let on = |session: Session| {
            holst_core::command::login::connect(
                Some(session),
                base.clone(),
                holst_core::system::clock::SystemClock,
                holst_core::system::id::RandomIds,
            )
            .map(|client| client.with_transport(std::sync::Arc::clone(&server)))
        };
        let login = || Session::new(SecretString::from("token"));
        let named = on(login().with_account_id(id))?;
        let unnamed = on(login())?;
        let owners = [
            whose_cache(LogoutMode::LocalOnly, Some(&named)).await,
            whose_cache(LogoutMode::LocalOnly, Some(&unnamed)).await,
            whose_cache(LogoutMode::Everywhere, None).await,
        ];
        assert_eq!(owners, [Some(id.to_owned()), None, None]);
        assert!(server.seen().is_empty(), "{:?}", server.seen());
        let asked = whose_cache(LogoutMode::Everywhere, Some(&unnamed)).await;
        assert_eq!(asked.as_deref(), Some(id));
        Ok(())
    }

    #[test]
    fn the_logout_outcome_names_the_server_and_the_stores() {
        let logged_out = |server, forgotten: &[&'static str], kept| LogoutReport {
            server,
            forgotten: forgotten.to_vec(),
            kept,
        };
        assert_eq!(
            logged_out(ServerSessionReport::Ended, &["keychain", "file"], false)
                .to_string(),
            "ended the session on the server\nforgot the login in: keychain, file"
        );
        assert_eq!(
            logged_out(ServerSessionReport::Skipped, &[], false).to_string(),
            "there was no saved login"
        );
        assert_eq!(
            serde_json::to_value(logged_out(
                ServerSessionReport::AlreadyEnded,
                &["file"],
                false
            ))
            .ok(),
            Some(json!({ "server": "already-ended", "forgotten": ["file"] }))
        );
    }
}
