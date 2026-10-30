//! The server's login as a state machine; deciding is pure, I/O is the

use std::time::Duration;

use holst_core::domain::session::{Session, SessionStatus};

pub(crate) trait LoginClient: Clone {
    /// Where a login is kept; the machine only hands it back to the caller.
    type Store: Copy;

    /// Whether the client still signs requests: the server has not refused it.
    fn status(&self) -> SessionStatus;
}

pub(crate) type Milliseconds = u64;

pub(crate) const RECHECK: Duration = Duration::from_secs(30);

pub(crate) const REVOKED_RECHECK: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct ReadyLogin<C: LoginClient> {
    pub(crate) session: Session,
    pub(crate) client: C,
    pub(crate) epoch: u64,
    pub(crate) checked_at: Milliseconds,
    pub(crate) store: C::Store,
}

#[derive(Clone)]
pub(crate) struct RevokedLogin {
    session: Session,
    checked_at: Milliseconds,
}

#[derive(Clone)]
pub(crate) enum LoginState<C: LoginClient> {
    Missing,
    Ready(ReadyLogin<C>),
    Revoked(RevokedLogin),
}

#[cfg_attr(test, derive(strum::IntoStaticStr), strum(serialize_all = "lowercase"))]
pub(crate) enum LoginStep {
    Use,
    Refuse,
    Reload,
}

pub(crate) struct NewLogin<C: LoginClient> {
    pub(crate) session: Session,
    pub(crate) store: C::Store,
}

#[cfg_attr(test, derive(strum::IntoStaticStr), strum(serialize_all = "lowercase"))]
pub(crate) enum LoginPlan<C: LoginClient> {
    Keep(ReadyLogin<C>),
    Refuse(RevokedLogin),
    Install(NewLogin<C>),
    Forget,
}

impl<C: LoginClient> ReadyLogin<C> {
    #[must_use]
    fn rechecked(&self, checked_at: Milliseconds, store: C::Store) -> Self {
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

impl<C: LoginClient> LoginState<C> {
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
    pub(crate) fn kept_unread(
        &self,
        checked_at: Milliseconds,
    ) -> Option<ReadyLogin<C>> {
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
        loaded: Option<(Session, C::Store)>,
        checked_at: Milliseconds,
    ) -> LoginPlan<C> {
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
    use holst_core::domain::session::{Session, SessionStatus};
    use secrecy::SecretString;

    use crate::login::machine::{LoginClient, LoginStep, Milliseconds, RevokedLogin};

    type LoginPlan = crate::login::machine::LoginPlan<Working>;
    type LoginState = crate::login::machine::LoginState<Working>;
    type ReadyLogin = crate::login::machine::ReadyLogin<Working>;

    #[derive(Clone)]
    struct Working;

    #[derive(Clone, Copy)]
    enum Place {
        Keychain,
        File,
    }

    impl LoginClient for Working {
        type Store = Place;

        fn status(&self) -> SessionStatus {
            SessionStatus::Active
        }
    }

    const SECOND: Milliseconds = 1_000;

    fn session(token: &str) -> Session {
        Session::new(SecretString::from(token))
    }

    fn in_keychain(token: &str) -> (Session, Place) {
        (session(token), Place::Keychain)
    }

    fn ready(token: &str, checked: Milliseconds) -> ReadyLogin {
        ReadyLogin {
            session: session(token),
            client: Working,
            epoch: 1,
            checked_at: checked,
            store: Place::Keychain,
        }
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
    fn ready_is_used_until_recheck() {
        let login = LoginState::Ready(ready("a", 0));
        assert_eq!(name(&login.next(29 * SECOND)), "use");
        assert_eq!(name(&login.next(30 * SECOND)), "reload");
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
    fn clock_going_back_counts_as_stale() {
        assert_eq!(
            name(&LoginState::Ready(ready("a", 50 * SECOND)).next(0)),
            "reload"
        );
        let revoked = LoginState::Revoked(RevokedLogin {
            session: session("a"),
            checked_at: 3_610 * SECOND,
        });
        assert_eq!(name(&revoked.next(10 * SECOND)), "reload");
        assert_eq!(name(&revoked.next(3_612 * SECOND)), "refuse");
    }

    #[test]
    fn plan_follows_the_store() {
        let ready = LoginState::Ready(ready("a", 0));
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
    }

    #[test]
    fn plan_records_the_check_time() {
        let login = LoginState::Ready(ready("a", 0));
        let plan = login.plan(Some((session("a"), Place::File)), 7 * SECOND);
        assert_eq!(kept_at(plan), Some(7 * SECOND));
    }
}
