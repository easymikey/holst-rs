pub(crate) mod local_data;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use holst_core::{
    command::table::CommandRisk,
    domain::session::Session,
    effect::settings::{CacheSettings, SessionLocation, Settings},
    remote::client::Client,
    storage::{
        cache::account::AccountDisk,
        session_store::{LoginRemoval, LoginSaveReport, SessionStoreKind},
    },
};

use crate::{
    error::{Error, Result},
    output::{self, OutputFormat},
    terminal::{
        awaiting::{apart, optional},
        detect::{StreamKind, TerminalType},
        signals::{self, Signal},
    },
};

/// Whether a held command may ask a person, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prompting {
    Keys,
    Lines,
    Off,
}

impl Prompting {
    /// `streams` are stdin, stdout and stderr; `disabled` is `HOLST_NO_PROMPT`
    /// set to a non-empty value.
    #[must_use]
    pub(crate) const fn of(
        streams: [StreamKind; 3],
        term_type: TerminalType,
        disabled: bool,
        format: OutputFormat,
    ) -> Self {
        match (streams, format, disabled, term_type) {
            (
                [
                    StreamKind::Terminal,
                    StreamKind::Terminal,
                    StreamKind::Terminal,
                ],
                OutputFormat::Text,
                false,
                TerminalType::Capable,
            ) => Prompting::Keys,
            (
                [
                    StreamKind::Terminal,
                    StreamKind::Terminal,
                    StreamKind::Terminal,
                ],
                OutputFormat::Text,
                false,
                TerminalType::Dumb,
            ) => Prompting::Lines,
            _ => Prompting::Off,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RunSettings {
    settings: Settings,
    home: Option<PathBuf>,
    terminal_type: TerminalType,
    prompting: Prompting,
    risk: CommandRisk,
}

impl RunSettings {
    pub(crate) const fn new(
        settings: Settings,
        home: Option<PathBuf>,
        terminal_type: TerminalType,
        prompting: Prompting,
    ) -> Self {
        Self {
            settings,
            home,
            terminal_type,
            prompting,
            risk: CommandRisk::Read,
        }
    }

    /// The settings of a command of `risk`: a write looks names up in trees
    /// read within the hour.
    #[must_use]
    pub(crate) fn for_risk(self, risk: CommandRisk) -> Self {
        Self { risk, ..self }
    }

    pub(crate) fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// The user's Downloads folder path, if there is a home.
    pub(crate) fn downloads_dir(&self) -> Option<PathBuf> {
        self.home().map(|home| home.join("Downloads"))
    }

    /// The user's Downloads folder, made if it is not there.
    pub(crate) fn downloads(&self) -> Result<PathBuf> {
        let dir = self.downloads_dir().ok_or(Error::NoDownloads)?;
        holst_core::storage::file::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub(crate) const fn terminal_type(&self) -> TerminalType {
        self.terminal_type
    }

    pub(crate) const fn prompting(&self) -> Prompting {
        self.prompting
    }

    pub(crate) fn location(&self) -> SessionLocation {
        self.settings.location.clone()
    }

    pub(crate) async fn load(&self) -> Result<Option<(Session, SessionStoreKind)>> {
        let location = self.location();
        apart(move || location.load())
            .await
            .ok_or(Error::SessionStoreLost)?
            .map_err(Error::from)
    }

    pub(crate) async fn save(&self, session: Session) -> Result<LoginSaveReport> {
        let location = self.location();
        apart(move || location.save(&session))
            .await
            .ok_or(Error::SessionStoreLost)?
            .map_err(Error::from)
    }

    pub(crate) async fn resave(
        &self,
        session: Session,
        store: SessionStoreKind,
    ) -> Result<LoginSaveReport> {
        let location = self.location();
        apart(move || location.resave(&session, store))
            .await
            .ok_or(Error::SessionStoreLost)?
            .map_err(Error::from)
    }

    pub(crate) async fn clear(
        &self,
        ended: Option<Session>,
    ) -> Result<Vec<(SessionStoreKind, holst_core::Result<LoginRemoval>)>> {
        let location = self.location();
        apart(move || location.clear(ended.as_ref()))
            .await
            .ok_or(Error::SessionStoreLost)
    }

    pub(crate) const fn cache(&self) -> &CacheSettings {
        &self.settings.cache
    }

    pub(crate) fn client(&self, session: Option<Session>) -> Result<Client> {
        let base = &self.settings.base_url;
        let client = holst_core::command::login::connect(
            session.map(|login| login.on(base)).transpose()?,
            base.clone(),
            holst_core::system::clock::SystemClock,
            holst_core::system::id::RandomIds,
        )?;
        let kept = match AccountDisk::from_settings(self.cache()) {
            Some(disk) => client.with_account_cache(Arc::new(disk)),
            None => client,
        };
        Ok(match self.risk {
            CommandRisk::Read => kept,
            CommandRisk::Write | CommandRisk::Destroy => kept.for_change(),
        })
    }
}

#[derive(Debug)]
struct SignedClient {
    client: Client,
    store: SessionStoreKind,
}

impl SignedClient {
    async fn load(run: &RunSettings) -> Result<Self> {
        let (session, store) =
            run.load()
                .await?
                .ok_or(Error::Holst(holst_core::Error::Login(
                    holst_core::error::login::LoginError::NoSavedLogin,
                )))?;
        Ok(Self {
            client: run.client(Some(session))?,
            store,
        })
    }

    async fn run(
        &self,
        work: impl AsyncFnOnce(&Client) -> Result<()>,
        interrupt: impl Future<Output = Signal>,
        save: impl AsyncFnOnce(Session, SessionStoreKind) -> Result<LoginSaveReport>,
    ) -> Result<()> {
        let result = signals::or_stopped(work(&self.client), interrupt).await;
        warn_unsaved(save_rotated(&self.client, self.store, save).await);
        result
    }
}

pub(crate) async fn save_rotated(
    client: &Client,
    store: SessionStoreKind,
    save: impl AsyncFnOnce(Session, SessionStoreKind) -> Result<LoginSaveReport>,
) -> Result<Option<LoginSaveReport>> {
    let saving = client.rotated().map(|session| async move {
        save(session.clone(), store)
            .await
            .inspect(|_| client.confirm_saved(&session))
    });
    optional(saving).await.transpose()
}

pub(crate) fn warn_unsaved(saved: Result<Option<LoginSaveReport>>) {
    if let Some((headline, err)) = unsaved_warning(saved) {
        output::warn_with(headline, &err);
    }
}

fn unsaved_warning(
    saved: Result<Option<LoginSaveReport>>,
) -> Option<(&'static str, Error)> {
    match saved {
        Ok(report) => report
            .and_then(|report| report.stale)
            .map(|stale| ("the refreshed login was saved", Error::Holst(stale))),
        Err(Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::LoginReplaced,
        ))) => None,
        Err(err) => Some(("the refreshed login was not saved", err)),
    }
}

pub(crate) async fn run_signed_in(
    run: &RunSettings,
    work: impl AsyncFnOnce(&Client) -> Result<()>,
) -> Result<()> {
    let loaded =
        signals::or_stopped(SignedClient::load(run), signals::stopped()).await?;
    loaded
        .run(work, signals::stopped(), async |session, store| {
            run.resave(session, store).await
        })
        .await
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::cell::Cell;

    use holst_core::{
        command::table::CommandRisk,
        domain::{cookie::CookieJar, session::Session},
        remote::client::Client,
        storage::session_store::{LoginSaveReport, SessionStoreKind},
    };
    use rstest::rstest;
    use secrecy::SecretString;

    use crate::{
        error::{Error, Result},
        output::OutputFormat,
        run::{Prompting, SignedClient, save_rotated, unsaved_warning},
        terminal::{
            detect::{StreamKind, TerminalType},
            signals::Signal,
        },
    };

    const TTY: [StreamKind; 3] = [StreamKind::Terminal; 3];

    #[rstest]
    #[case::keys(
        TTY,
        TerminalType::Capable,
        false,
        OutputFormat::Text,
        Prompting::Keys
    )]
    #[case::lines(TTY, TerminalType::Dumb, false, OutputFormat::Text, Prompting::Lines)]
    #[case::disabled(
        TTY,
        TerminalType::Capable,
        true,
        OutputFormat::Text,
        Prompting::Off
    )]
    #[case::json(TTY, TerminalType::Capable, false, OutputFormat::Json, Prompting::Off)]
    #[case::pretty(
        TTY,
        TerminalType::Capable,
        false,
        OutputFormat::Pretty,
        Prompting::Off
    )]
    #[case::stdin_piped(
        [StreamKind::Redirected, StreamKind::Terminal, StreamKind::Terminal],
        TerminalType::Capable,
        false,
        OutputFormat::Text,
        Prompting::Off
    )]
    #[case::stdout_piped(
        [StreamKind::Terminal, StreamKind::Redirected, StreamKind::Terminal],
        TerminalType::Capable,
        false,
        OutputFormat::Text,
        Prompting::Off
    )]
    #[case::stderr_piped(
        [StreamKind::Terminal, StreamKind::Terminal, StreamKind::Redirected],
        TerminalType::Capable,
        false,
        OutputFormat::Text,
        Prompting::Off
    )]
    fn a_prompt_needs_terminals_text_and_no_opt_out(
        #[case] streams: [StreamKind; 3],
        #[case] term_type: TerminalType,
        #[case] disabled: bool,
        #[case] format: OutputFormat,
        #[case] expected: Prompting,
    ) {
        assert_eq!(
            Prompting::of(streams, term_type, disabled, format),
            expected
        );
    }

    #[test]
    fn a_replaced_login_is_not_a_warning() {
        let replaced = Err(Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::LoginReplaced,
        )));
        assert!(unsaved_warning(replaced).is_none());
        let locked = Err(Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::KeychainLocked,
        )));
        assert!(unsaved_warning(locked).is_some());
    }

    fn signed(store: SessionStoreKind) -> Result<SignedClient> {
        let base: holst_core::domain::url::BaseUrl = "https://app.holst.so".parse()?;
        let session =
            Session::with_cookies(SecretString::from("tok"), CookieJar::default());
        let client = holst_core::command::login::connect(
            Some(session),
            base,
            holst_core::system::clock::SystemClock,
            holst_core::system::id::RandomIds,
        )?
        .with_token(SecretString::from("fresh"));
        Ok(SignedClient { client, store })
    }

    fn saved_in(kind: SessionStoreKind) -> LoginSaveReport {
        LoginSaveReport {
            store: kind,
            stale: None,
        }
    }

    #[tokio::test]
    async fn rotated_session_is_saved_once() -> Result<()> {
        let saved = Cell::new(0);
        let save = async |_: Session, store: SessionStoreKind| {
            saved.set(saved.get() + 1);
            Ok(saved_in(store))
        };
        let stored = signed(SessionStoreKind::Keychain)?;
        save_rotated(&stored.client, stored.store, save).await?;
        save_rotated(&stored.client, stored.store, save).await?;
        assert_eq!(saved.get(), 1, "saved again without a change");
        Ok(())
    }

    #[rstest::rstest]
    #[case::file(SessionStoreKind::File)]
    #[case::keychain(SessionStoreKind::Keychain)]
    #[tokio::test]
    async fn rotation_is_saved_where_it_was_loaded_from(
        #[case] store: SessionStoreKind,
    ) -> Result<()> {
        let asked = Cell::new(None);
        let stored = signed(store)?;
        let saved = save_rotated(
            &stored.client,
            stored.store,
            async |_: Session, to: SessionStoreKind| {
                asked.set(Some(to));
                Ok(saved_in(to))
            },
        )
        .await?;
        assert_eq!(
            (asked.get(), saved.map(|saved| saved.store)),
            (Some(store), Some(store))
        );
        Ok(())
    }

    #[rstest::rstest]
    #[case::interrupt(Signal::Interrupt)]
    #[case::terminate(Signal::Terminate)]
    #[case::hangup(Signal::HangUp)]
    #[tokio::test]
    async fn a_stopped_command_still_saves_the_rotated_session(
        #[case] signal: Signal,
    ) -> Result<()> {
        let saved = Cell::new(0);
        let save = async |_: Session, store: SessionStoreKind| {
            saved.set(saved.get() + 1);
            Ok(saved_in(store))
        };
        let result = signed(SessionStoreKind::Keychain)?
            .run(
                async |_: &Client| std::future::pending::<Result<()>>().await,
                std::future::ready(signal),
                save,
            )
            .await;
        assert!(result.is_err(), "{result:?}");
        assert_eq!(saved.get(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn saving_error_is_reported() -> Result<()> {
        let stored = signed(SessionStoreKind::Keychain)?;
        let err = save_rotated(
            &stored.client,
            stored.store,
            async |_: Session, _: SessionStoreKind| {
                Err(holst_core::Error::LocalData(
                    holst_core::error::local_data::LocalDataError::NoHome,
                )
                .into())
            },
        )
        .await
        .err();
        assert!(matches!(
            err,
            Some(Error::Holst(holst_core::Error::LocalData(
                holst_core::error::local_data::LocalDataError::NoHome
            )))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn saving_failure_does_not_fail_a_done_command() -> Result<()> {
        let worked = Cell::new(false);
        let result = signed(SessionStoreKind::File)?
            .run(
                async |_: &Client| {
                    worked.set(true);
                    Ok(())
                },
                std::future::pending::<Signal>(),
                async |_: Session, _: SessionStoreKind| {
                    Err(holst_core::Error::LocalData(
                        holst_core::error::local_data::LocalDataError::NoHome,
                    )
                    .into())
                },
            )
            .await;
        assert!(result.is_ok() && worked.get(), "{result:?}");
        Ok(())
    }

    #[rstest]
    #[case::read(CommandRisk::Read, false)]
    #[case::write(CommandRisk::Write, true)]
    #[case::destroy(CommandRisk::Destroy, true)]
    fn only_a_read_keeps_the_client_off_the_change_rule(
        #[case] risk: CommandRisk,
        #[case] change: bool,
    ) -> Result<()> {
        let base = holst_core::domain::url::BaseUrl::or_default(None)?;
        let location = holst_core::effect::settings::SessionLocation::Dir("x".into());
        let run = super::RunSettings::new(
            holst_core::effect::settings::Settings {
                base_url: base,
                location,
                log: holst_core::effect::settings::LogSettings::default(),
                cache: holst_core::effect::settings::CacheSettings::default(),
            },
            None,
            TerminalType::Capable,
            Prompting::Off,
        )
        .for_risk(risk);
        assert_eq!(run.client(None)?.runs_change(), change);
        Ok(())
    }
}
