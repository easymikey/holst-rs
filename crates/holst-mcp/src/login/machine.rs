//! The server's login as a state machine; deciding is pure, I/O is the

use std::time::Duration;

use holst_core::{
    domain::session::{Session, SessionStatus, SessionStoreKind},
    remote::client::Client,
};

pub(crate) type Milliseconds = u64;

pub(crate) const RECHECK: Duration = Duration::from_secs(30);

pub(crate) const REVOKED_RECHECK: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct ReadyLogin {
    pub(crate) session: Session,
    pub(crate) client: Client,
    pub(crate) epoch: u64,
    pub(crate) checked_at: Milliseconds,
    pub(crate) store: SessionStoreKind,
}

#[derive(Clone)]
pub(crate) struct RevokedLogin {
    session: Session,
    checked_at: Milliseconds,
}

#[derive(Clone)]
pub(crate) enum LoginState {
    Missing,
    Ready(ReadyLogin),
    Revoked(RevokedLogin),
}

#[cfg_attr(test, derive(strum::IntoStaticStr), strum(serialize_all = "lowercase"))]
pub(crate) enum LoginStep {
    Use,
    Refuse,
    Reload,
}

pub(crate) struct NewLogin {
    pub(crate) session: Session,
    pub(crate) store: SessionStoreKind,
}

#[cfg_attr(test, derive(strum::IntoStaticStr), strum(serialize_all = "lowercase"))]
pub(crate) enum LoginPlan {
    Keep(ReadyLogin),
    Refuse(RevokedLogin),
    Install(NewLogin),
    Forget,
}

impl ReadyLogin {
    #[must_use]
    fn rechecked(&self, checked_at: Milliseconds, store: SessionStoreKind) -> Self {
        Self {
            checked_at,
            store,
            ..self.clone()
        }
    }

    #[must_use]
    fn revoke(&self, checked_at: Milliseconds) -> RevokedLogin {
        RevokedLogin {
            session: self.session.clone(),
            checked_at,
        }
    }

    #[must_use]
    pub(crate) fn rotated(&self, session: Session) -> Self {
        Self {
            session,
            ..self.clone()
        }
    }

    fn is_active(&self) -> bool {
        self.client.status() == SessionStatus::Active
    }
}

impl RevokedLogin {
    #[must_use]
    fn rechecked(&self, checked_at: Milliseconds) -> Self {
        Self {
            checked_at,
            ..self.clone()
        }
    }
}

fn within(since: Milliseconds, checked_at: Milliseconds, window: Duration) -> bool {
    checked_at
        .checked_sub(since)
        .is_some_and(|age| u128::from(age) < window.as_millis())
}

impl LoginState {
    #[must_use]
    pub(crate) fn next(&self, checked_at: Milliseconds) -> LoginStep {
        match self {
            LoginState::Ready(ready)
                if ready.is_active()
                    && within(ready.checked_at, checked_at, RECHECK) =>
            {
                LoginStep::Use
            }
            LoginState::Revoked(revoked)
                if within(revoked.checked_at, checked_at, REVOKED_RECHECK) =>
            {
                LoginStep::Refuse
            }
            LoginState::Missing | LoginState::Ready(_) | LoginState::Revoked(_) => {
                LoginStep::Reload
            }
        }
    }

    #[must_use]
    pub(crate) fn epoch(&self) -> Option<u64> {
        match self {
            LoginState::Ready(ready) => Some(ready.epoch),
            LoginState::Missing | LoginState::Revoked(_) => None,
        }
    }

    /// A store that cannot be read does not end a working login.
    #[must_use]
    pub(crate) fn kept_unread(&self, checked_at: Milliseconds) -> Option<ReadyLogin> {
        match self {
            LoginState::Ready(ready) if ready.is_active() => {
                Some(ready.rechecked(checked_at, ready.store))
            }
            LoginState::Missing | LoginState::Ready(_) | LoginState::Revoked(_) => None,
        }
    }

    #[must_use]
    pub(crate) fn plan(
        &self,
        loaded: Option<(Session, SessionStoreKind)>,
        checked_at: Milliseconds,
    ) -> LoginPlan {
        match (self, loaded) {
            (_, None) => LoginPlan::Forget,
            (LoginState::Ready(ready), Some((session, store)))
                if session == ready.session && ready.is_active() =>
            {
                LoginPlan::Keep(ready.rechecked(checked_at, store))
            }
            (LoginState::Ready(ready), Some((session, _)))
                if session == ready.session =>
            {
                LoginPlan::Refuse(ready.revoke(checked_at))
            }
            (LoginState::Revoked(revoked), Some((session, _)))
                if session == revoked.session =>
            {
                LoginPlan::Refuse(revoked.rechecked(checked_at))
            }
            (
                LoginState::Missing | LoginState::Ready(_) | LoginState::Revoked(_),
                Some((session, store)),
            ) => LoginPlan::Install(NewLogin { session, store }),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use holst_core::domain::session::{Session, SessionStoreKind};

    use crate::login::{
        fake::{self, Script},
        machine::{
            LoginPlan,
            LoginState,
            LoginStep,
            Milliseconds,
            ReadyLogin,
            RevokedLogin,
        },
    };

    const SECOND: Milliseconds = 1_000;

    fn session(token: &str) -> Session {
        fake::session(token)
    }

    fn in_keychain(token: &str) -> (Session, SessionStoreKind) {
        (session(token), SessionStoreKind::Keychain)
    }

    fn ready(token: &str, checked: Milliseconds) -> holst_core::Result<ReadyLogin> {
        Ok(ReadyLogin {
            session: session(token),
            client: fake::client(session(token), &Script::new([]))?,
            epoch: 1,
            checked_at: checked,
            store: SessionStoreKind::Keychain,
        })
    }

    fn name(next: &LoginStep) -> &'static str {
        next.into()
    }

    fn kind(plan: &LoginPlan) -> &'static str {
        plan.into()
    }

    fn kept_at(plan: LoginPlan) -> Option<Milliseconds> {
        match plan {
            LoginPlan::Keep(ready) => Some(ready.checked_at),
            LoginPlan::Refuse(_) | LoginPlan::Install(_) | LoginPlan::Forget => None,
        }
    }

    #[test]
    fn ready_is_used_until_recheck() -> holst_core::Result<()> {
        let login = LoginState::Ready(ready("a", 0)?);
        assert_eq!(name(&login.next(29 * SECOND)), "use");
        assert_eq!(name(&login.next(30 * SECOND)), "reload");
        Ok(())
    }

    #[test]
    fn revoked_is_refused_then_rechecked() {
        let login = LoginState::Revoked(RevokedLogin {
            session: session("a"),
            checked_at: 10 * SECOND,
        });
        assert_eq!(name(&login.next(14 * SECOND)), "refuse");
        assert_eq!(name(&login.next(15 * SECOND)), "reload");
        assert_eq!(name(&LoginState::Missing.next(0)), "reload");
    }

    #[test]
    fn clock_going_back_counts_as_stale() -> holst_core::Result<()> {
        assert_eq!(
            name(&LoginState::Ready(ready("a", 50 * SECOND)?).next(0)),
            "reload"
        );
        let revoked = LoginState::Revoked(RevokedLogin {
            session: session("a"),
            checked_at: 3_610 * SECOND,
        });
        assert_eq!(name(&revoked.next(10 * SECOND)), "reload");
        assert_eq!(name(&revoked.next(3_612 * SECOND)), "refuse");
        Ok(())
    }

    #[test]
    fn plan_follows_the_store() -> holst_core::Result<()> {
        let ready = LoginState::Ready(ready("a", 0)?);
        let revoked = LoginState::Revoked(RevokedLogin {
            session: session("a"),
            checked_at: 0,
        });
        let cases = [
            (&ready, None, "forget"),
            (&ready, Some("a"), "keep"),
            (&ready, Some("b"), "install"),
            (&revoked, Some("a"), "refuse"),
            (&revoked, Some("b"), "install"),
            (&LoginState::Missing, Some("a"), "install"),
        ];
        let plan = |(login, stored, _): &(&LoginState, Option<&str>, &str)| {
            kind(&login.plan(stored.map(in_keychain), SECOND))
        };
        let got: Vec<_> = cases.iter().map(plan).collect();
        let want: Vec<_> = cases.iter().map(|&(.., want)| want).collect();
        assert_eq!(got, want);
        Ok(())
    }

    #[test]
    fn plan_records_the_check_time() -> holst_core::Result<()> {
        let login = LoginState::Ready(ready("a", 0)?);
        let plan = login.plan(Some((session("a"), SessionStoreKind::File)), 7 * SECOND);
        assert_eq!(kept_at(plan), Some(7 * SECOND));
        Ok(())
    }
}
