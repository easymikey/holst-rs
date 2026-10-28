#[cfg(test)] pub(crate) mod fake;
pub(crate) mod machine;
pub(crate) mod store;

use std::sync::{Arc, atomic::AtomicU64};

use arc_swap::ArcSwap;
use holst_core::{
    domain::session::{Session, SessionStatus},
    effect::clock::Clock,
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
};
use tokio::sync::Mutex;

use crate::{
    error::{Error, Result},
    login::{
        machine::{LoginClient, Milliseconds},
        store::Backend,
    },
};

pub(crate) type LoginState = machine::LoginState<Client>;

pub(crate) type LoginPlan = machine::LoginPlan<Client>;

pub(crate) type LoginStep<'a> = machine::LoginStep<'a, Client>;

pub(crate) type ReadyLogin = machine::ReadyLogin<Client>;

impl LoginClient for Client {
    type Store = SessionStoreKind;

    fn status(&self) -> SessionStatus {
        Self::status(self)
    }
}

pub(crate) struct SharedLogin {
    pub(crate) state: ArcSwap<LoginState>,
    pub(crate) gate: Mutex<()>,
    pub(crate) epochs: AtomicU64,
    pub(crate) backend: Arc<dyn Backend>,
    clock: Arc<dyn Clock>,
}

impl SharedLogin {
    pub(crate) fn new(backend: Arc<dyn Backend>, clock: Arc<dyn Clock>) -> Self {
        Self {
            state: ArcSwap::from_pointee(LoginState::Missing),
            gate: Mutex::new(()),
            epochs: AtomicU64::new(0),
            backend,
            clock,
        }
    }

    pub(crate) fn now(&self) -> Milliseconds {
        self.clock.unix_millis()
    }

    pub(crate) async fn load(&self) -> Result<Option<(Session, SessionStoreKind)>> {
        self.blocking(|backend| backend.load()).await
    }

    pub(crate) async fn save(
        &self,
        session: Session,
        from: SessionStoreKind,
    ) -> Result<LoginSaveReport> {
        self.blocking(move |backend| backend.save(&session, from))
            .await
    }

    async fn blocking<T: Send + 'static>(
        &self,
        step: impl FnOnce(&dyn Backend) -> holst_core::Result<T> + Send + 'static,
    ) -> Result<T> {
        let backend = Arc::clone(&self.backend);
        Ok(tokio::task::spawn_blocking(move || step(&*backend))
            .await
            .map_err(|_| Error::StoreAborted)??)
    }
}

#[derive(Clone)]
pub(crate) struct HandedClient {
    pub(crate) client: Client,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
}

impl HandedClient {
    pub(crate) fn new(ready: &ReadyLogin, generation: u64) -> Self {
        Self {
            client: ready.client.clone(),
            generation,
            epoch: ready.epoch,
        }
    }
}

pub(crate) async fn when<T>(work: Option<impl Future<Output = T>>) -> Option<T> {
    match work {
        Some(work) => Some(work.await),
        None => None,
    }
}

#[cfg(test)]
impl HandedClient {
    pub(crate) fn offline() -> Result<Self> {
        Ok(Self {
            client: holst_core::command::login::connect(
                None,
                "http://127.0.0.1:1".parse()?,
                holst_core::system::clock::SystemClock,
                holst_core::system::id::RandomIds,
            )?,
            generation: 0,
            epoch: 0,
        })
    }
}

impl std::ops::Deref for HandedClient {
    type Target = Client;

    fn deref(&self) -> &Client {
        &self.client
    }
}

pub(crate) fn hand_out(login: &LoginState, generation: u64) -> Result<HandedClient> {
    match login {
        LoginState::Ready(ready) => Ok(HandedClient::new(ready, generation)),
        LoginState::Revoked(_) => Err(holst_core::Error::Login(
            holst_core::error::login::LoginError::SessionRevoked,
        )
        .into()),
        LoginState::Missing => Err(holst_core::Error::Login(
            holst_core::error::login::LoginError::NoSavedLogin,
        )
        .into()),
    }
}
