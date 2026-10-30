//! The login store port and its real adapter.

use std::sync::Arc;

use holst_core::{
    domain::session::Session,
    effect::settings::Settings,
    remote::client::Client,
    storage::{
        cache::account::AccountDisk,
        session_store::{LoginSaveReport, SessionStoreKind},
    },
};

pub(crate) trait Backend: Send + Sync {
    /// The stored login and its source; an unreadable store is an error.
    fn load(&self) -> holst_core::Result<Option<(Session, SessionStoreKind)>>;

    /// Store a rotated login back in the store it came from.
    fn save(
        &self,
        session: &Session,
        from: SessionStoreKind,
    ) -> holst_core::Result<LoginSaveReport>;

    /// A client signed with `login`.
    fn connect(&self, session: Session) -> holst_core::Result<Client>;
}

#[derive(Debug, Clone)]
pub(crate) struct SystemBackend {
    settings: Settings,
}

impl SystemBackend {
    pub(crate) const fn new(settings: Settings) -> Self {
        Self { settings }
    }

    /// Store a new login in the first store that takes it (`holst-mcp setup`).
    pub(crate) fn save_new(
        &self,
        session: &Session,
    ) -> holst_core::Result<LoginSaveReport> {
        self.settings.location.save(session)
    }

    /// A client with no login, to log in with.
    pub(crate) fn anonymous(&self) -> holst_core::Result<Client> {
        self.client(None)
    }

    fn client(&self, session: Option<Session>) -> holst_core::Result<Client> {
        let base = &self.settings.base_url;
        let client = holst_core::command::login::connect(
            session.map(|login| login.on(base)).transpose()?,
            base.clone(),
            holst_core::system::clock::SystemClock,
            holst_core::system::id::RandomIds,
        )?;
        let disk = AccountDisk::from_settings(&self.settings.cache);
        Ok(disk.into_iter().fold(client, |client, disk| {
            client.with_account_cache(Arc::new(disk))
        }))
    }
}

impl Backend for SystemBackend {
    fn load(&self) -> holst_core::Result<Option<(Session, SessionStoreKind)>> {
        self.settings.location.load()
    }

    fn save(
        &self,
        session: &Session,
        from: SessionStoreKind,
    ) -> holst_core::Result<LoginSaveReport> {
        self.settings.location.resave(session, from)
    }

    fn connect(&self, session: Session) -> holst_core::Result<Client> {
        self.client(Some(session))
    }
}
