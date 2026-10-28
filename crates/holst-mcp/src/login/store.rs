//! The login store port and its real adapter.

use holst_core::{
    domain::session::Session,
    effect::settings::Settings,
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
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
        holst_core::command::login::connect(
            Some(session),
            self.settings.base_url.clone(),
            holst_core::system::clock::SystemClock,
            holst_core::system::id::RandomIds,
        )
    }
}
