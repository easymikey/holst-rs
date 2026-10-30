//! The setup dialog against the fake Holst server, through a scripted terminal.

#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "the login folder of a test is made and removed here"
)]

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use holst_core::{
    command::account::SessionCheck,
    domain::session::Session,
    effect::settings::Settings,
    error::hint::Surface,
    fake::{Fake, Fixture},
};
use holst_form::{
    error::FormError,
    fake::Script,
    form::Form,
    line::Keystroke,
    terminal::TerminalKind,
};

use crate::{
    login::store::{Backend as _, SystemBackend},
    setup::{
        Clients,
        Secret,
        SetupError,
        Steps,
        connect::tests::Machine,
        set_up,
        stale_warning,
    },
};

#[derive(Debug, thiserror::Error)]
enum Trouble {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Holst(#[from] holst_core::Error),

    #[error(transparent)]
    Setup(#[from] SetupError),
}

/// The fake server and a login store in a folder of its own.
struct Rig {
    _fake: Fake,
    backend: SystemBackend,
    home: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.home).unwrap_or_default();
    }
}

fn settings_at(home: &Path, base: &str) -> Result<Settings, holst_core::Error> {
    let variables = [
        ("HOLST_HOME", OsString::from(home)),
        ("HOLST_BASE_URL", OsString::from(base)),
    ];
    Settings::from_variables(|name| {
        variables
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    })
}

async fn rig(test: &str) -> Result<Rig, Trouble> {
    let fake = Fake::start(Fixture::default().workspace("Team")).await?;
    let home = std::env::temp_dir()
        .join(format!("holst-mcp-setup-{test}-{}", std::process::id()));
    std::fs::remove_dir_all(&home).unwrap_or_default();
    std::fs::create_dir_all(&home)?;
    let settings = settings_at(&home, fake.base_url())?;
    Ok(Rig {
        _fake: fake,
        backend: SystemBackend::new(settings),
        home,
    })
}

type Outcome = (Result<(), SetupError>, String, Vec<String>);

impl Rig {
    fn saved(&self) -> Result<Option<Session>, Trouble> {
        Ok(self.backend.load()?.map(|(session, _)| session))
    }

    /// Runs the steps on the script with no client program on the machine.
    async fn steps_on(&self, script: Script, steps: Steps) -> Outcome {
        self.steps_for(script, steps, &Clients::Ask).await
    }

    async fn steps_for(
        &self,
        script: Script,
        steps: Steps,
        clients: &Clients,
    ) -> Outcome {
        let form = Form::bare(script);
        let machine = Machine::with(&[]);
        let outcome = set_up(
            &self.backend,
            &form,
            &machine,
            "/opt/holst-mcp",
            steps,
            clients,
        )
        .await;
        (outcome, form.terminal().written(), machine.calls.take())
    }

    /// Runs the steps on answers typed as lines (a terminal without keys).
    async fn steps(&self, typed: &[&'static str], steps: Steps) -> Outcome {
        let script = Script::new(TerminalKind::Dumb, [], typed.iter().copied());
        self.steps_on(script, steps).await
    }

    /// Runs the login on `typed`; returns the outcome and the terminal text.
    async fn setup(&self, typed: &[&'static str]) -> (Result<(), SetupError>, String) {
        let (outcome, shown, _) = self.steps(typed, Steps::LoginOnly).await;
        (outcome, shown)
    }
}

const EMAIL: &str = "ada@example.com";
const LOGGED_IN: &str = "Logged in";

#[tokio::test]
async fn a_right_code_saves_the_login_the_server_reads() -> Result<(), Trouble> {
    let rig = rig("success").await?;
    let (outcome, shown) = rig.setup(&[EMAIL, "123456"]).await;
    outcome?;
    assert!(shown.contains("A code was sent to"), "{shown}");
    assert!(shown.contains(LOGGED_IN), "{shown}");
    assert!(
        shown.contains("picks the login up on its next call"),
        "{shown}"
    );
    assert!(!shown.contains("123456"), "{shown}");
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_wrong_code_asks_again_and_a_later_right_one_saves() -> Result<(), Trouble> {
    let rig = rig("again").await?;
    let (outcome, shown) = rig
        .setup(&["not an address", EMAIL, "000000", "111111", "123456"])
        .await;
    outcome?;
    assert!(
        shown.contains("enter an address like you@example.com"),
        "{shown}"
    );
    assert_eq!(
        shown.matches("that code did not match").count(),
        2,
        "{shown}"
    );
    assert!(shown.contains(LOGGED_IN), "{shown}");
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn an_empty_code_is_asked_again_without_a_try() -> Result<(), Trouble> {
    let rig = rig("empty").await?;
    let (outcome, shown) = rig.setup(&[EMAIL, "", "123456"]).await;
    outcome?;
    assert_eq!(shown.matches("this cannot be empty").count(), 1, "{shown}");
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn three_wrong_codes_fail_and_save_nothing() -> Result<(), Trouble> {
    let rig = rig("refused").await?;
    let (outcome, shown) = rig
        .setup(&[EMAIL, "000000", "111111", "222222", "123456"])
        .await;
    assert!(matches!(outcome, Err(SetupError::Refused(Secret::Code))));
    assert_eq!(
        shown.matches("that code did not match").count(),
        2,
        "{shown}"
    );
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn a_padded_email_is_trimmed() -> Result<(), Trouble> {
    let rig = rig("padded").await?;
    let (outcome, shown) = rig.setup(&["  ada@example.com  ", "123456"]).await;
    outcome?;
    assert!(!shown.contains("enter an address"), "{shown}");
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_piped_wrong_code_is_refused_without_asking_again() -> Result<(), Trouble> {
    let rig = rig("piped").await?;
    let script = Script::new(TerminalKind::Redirected, [], [EMAIL, "000000", "123456"]);
    let (outcome, _, _) = rig.steps_on(script, Steps::LoginOnly).await;
    assert!(matches!(outcome, Err(SetupError::Refused(Secret::Code))));
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn an_existing_login_stays_unless_the_answer_is_yes() -> Result<(), Trouble> {
    let rig = rig("existing").await?;
    rig.setup(&[EMAIL, "123456"]).await.0?;
    let before = rig.saved()?;
    let (no, asked) = rig.setup(&["n"]).await;
    no?;
    assert!(asked.contains("Log in again?"), "{asked}");
    assert!(!asked.contains("Email"), "{asked}");
    assert_eq!(rig.saved()?, before);
    let (blank, left_blank) = rig.setup(&[""]).await;
    blank?;
    assert!(!left_blank.contains("Email"), "{left_blank}");
    let (yes, again) = rig.setup(&["y", EMAIL, "123456"]).await;
    yes?;
    assert!(again.contains(LOGGED_IN), "{again}");
    Ok(())
}

#[tokio::test]
async fn the_end_of_input_stops_the_dialog() -> Result<(), Trouble> {
    let rig = rig("eof").await?;
    let (outcome, _) = rig.setup(&[EMAIL]).await;
    assert!(matches!(
        outcome,
        Err(SetupError::Form(FormError::Cancelled))
    ));
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn typed_keys_never_show_the_code() -> Result<(), Trouble> {
    let rig = rig("keys").await?;
    let keys = [EMAIL, "123456"]
        .into_iter()
        .flat_map(|text| text.chars().map(Keystroke::Char).chain([Keystroke::Enter]));
    let (outcome, shown, _) = rig
        .steps_on(Script::new(TerminalKind::Live, keys, []), Steps::LoginOnly)
        .await;
    outcome?;
    assert!(!shown.contains("123456"), "{shown:?}");
    assert!(shown.contains("▪▪▪▪▪▪"), "{shown:?}");
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn no_connect_logs_in_and_leaves_the_clients_alone() -> Result<(), Trouble> {
    let rig = rig("no-connect").await?;
    let (outcome, shown, asked) = rig.steps(&[EMAIL, "123456"], Steps::LoginOnly).await;
    outcome?;
    assert!(shown.contains(LOGGED_IN), "{shown}");
    assert!(asked.is_empty(), "{asked:?}");
    Ok(())
}

#[tokio::test]
async fn connect_only_skips_the_login() -> Result<(), Trouble> {
    let rig = rig("connect-only").await?;
    let (outcome, shown, asked) = rig.steps(&[], Steps::ConnectOnly).await;
    outcome?;
    assert!(
        shown.contains("Other clients: add the server by hand."),
        "{shown}"
    );
    assert_eq!(asked, ["claude mcp get holst-rs", "codex mcp get holst-rs"]);
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn both_steps_log_in_then_connect() -> Result<(), Trouble> {
    let rig = rig("both").await?;
    let (outcome, shown, asked) = rig.steps(&[EMAIL, "123456"], Steps::Both).await;
    outcome?;
    assert!(shown.contains(LOGGED_IN), "{shown}");
    assert!(shown.contains("\"mcpServers\""), "{shown}");
    assert_eq!(asked, ["claude mcp get holst-rs", "codex mcp get holst-rs"]);
    Ok(())
}

#[tokio::test]
async fn named_clients_without_a_terminal_connect_and_never_ask_to_log_in()
-> Result<(), Trouble> {
    let rig = rig("scripted").await?;
    rig.setup(&[EMAIL, "123456"]).await.0?;
    let before = rig.saved()?;
    let script = Script::new(TerminalKind::Redirected, [], []);
    let (outcome, shown, asked) = rig
        .steps_for(script, Steps::Both, &Clients::parse("claude")?)
        .await;
    outcome?;
    assert!(!shown.contains("Log in again"), "{shown}");
    assert!(!shown.contains("Email"), "{shown}");
    assert_eq!(
        asked.first().map(String::as_str),
        Some("claude mcp get holst-rs")
    );
    assert_eq!(rig.saved()?, before);
    Ok(())
}

#[tokio::test]
async fn named_clients_without_a_terminal_or_a_login_say_where_to_log_in()
-> Result<(), Trouble> {
    let rig = rig("scripted-bare").await?;
    let script = Script::new(TerminalKind::Redirected, [], []);
    let (outcome, shown, asked) = rig
        .steps_for(script, Steps::Both, &Clients::parse("claude")?)
        .await;
    assert_eq!(
        outcome.err().map(|err| err.to_string()).as_deref(),
        Some("There is no saved login: run holst-mcp setup in a terminal.")
    );
    assert!(!shown.contains("Email"), "{shown}");
    assert!(asked.is_empty(), "{asked:?}");
    Ok(())
}

#[test]
fn a_stale_copy_is_warned_about_with_its_hint() {
    let stale = holst_core::Error::LocalData(
        holst_core::error::local_data::LocalDataError::StaleLogin {
            session_store: "keychain",
            error: Box::new(holst_core::Error::Login(
                holst_core::error::login::LoginError::NoSavedLogin,
            )),
        },
    );
    let warning = stale_warning(&stale);
    assert!(
        warning.starts_with("an older login may be left in the keychain"),
        "{warning}"
    );
    assert!(
        warning.contains("run `holst-mcp setup` in a terminal again"),
        "{warning}"
    );
}

#[tokio::test]
async fn a_second_setup_ends_the_first_server_session() -> Result<(), Trouble> {
    let rig = rig("relogin").await?;
    rig.setup(&[EMAIL, "123456"]).await.0?;
    let old = rig
        .saved()?
        .ok_or(holst_core::error::login::LoginError::NoSavedLogin)
        .map_err(holst_core::Error::from)?;
    rig.setup(&["y", EMAIL, "123456"]).await.0?;
    let client = rig.backend.connect(old)?;
    let check = holst_core::command::account::check(&client).await;
    assert!(
        matches!(check, SessionCheck::Revoked(_)),
        "the replaced login is still alive on the server: {check:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_login_saved_for_another_server_is_refused_to_the_agent()
-> Result<(), Trouble> {
    let rig = rig("other-server").await?;
    rig.setup(&[EMAIL, "123456"]).await.0?;
    let staging =
        SystemBackend::new(settings_at(&rig.home, "https://staging.example")?);
    let saved = staging.load()?.map(|(session, _)| session);
    let refused = saved
        .map(|session| staging.connect(session))
        .and_then(Result::err);
    let advice = refused
        .as_ref()
        .and_then(holst_core::Error::hint)
        .map(|hint| hint.on(Surface::Mcp).to_string());
    assert!(refused.is_some_and(|err| {
        err.to_string()
            .starts_with("the saved login is for http://")
    }));
    assert_eq!(
        advice.as_deref(),
        Some(
            "ask the user to run `holst-mcp setup` in a terminal to log in again, then call again"
        )
    );
    Ok(())
}
