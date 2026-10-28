//! The setup dialog against the fake Holst server, through in-memory lines.

#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "the login folder of a test is made and removed here"
)]

use std::{cell::RefCell, ffi::OsString, io, path::PathBuf};

use holst_core::{
    domain::session::Session,
    effect::settings::Settings,
    fake::{Fake, Fixture},
};

use crate::{
    login::store::{Backend as _, SystemBackend},
    setup::{Program, Secret, SetupError, Steps, Talk, connect::Ran, run, set_up},
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

async fn rig(test: &str) -> Result<Rig, Trouble> {
    let fake = Fake::start(Fixture::default().workspace("Team")).await?;
    let home = std::env::temp_dir()
        .join(format!("holst-mcp-setup-{test}-{}", std::process::id()));
    std::fs::remove_dir_all(&home).unwrap_or_default();
    std::fs::create_dir_all(&home)?;
    let variables = [
        ("HOLST_HOME", OsString::from(&home)),
        ("HOLST_BASE_URL", OsString::from(fake.base_url())),
    ];
    let settings = Settings::from_variables(|name| {
        variables
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    })?;
    Ok(Rig {
        _fake: fake,
        backend: SystemBackend::new(settings),
        home,
    })
}

impl Rig {
    fn saved(&self) -> Result<Option<Session>, Trouble> {
        Ok(self.backend.load()?.map(|(session, _)| session))
    }

    /// Runs the given steps on `typed`, with no client program on the machine.
    async fn steps(
        &self,
        typed: &str,
        steps: Steps,
    ) -> (Result<(), SetupError>, String, Vec<String>) {
        let mut talk = Talk::new(typed.as_bytes(), Vec::new(), false);
        let programs = Absent::default();
        let outcome =
            set_up(&self.backend, &mut talk, &programs, "/opt/holst-mcp", steps).await;
        (
            outcome,
            String::from_utf8_lossy(&talk.out).into_owned(),
            programs.asked.into_inner(),
        )
    }

    /// Runs the dialog on `typed`; returns the outcome and the terminal text.
    async fn setup(&self, typed: &str) -> (Result<(), SetupError>, String) {
        let mut talk = Talk::new(typed.as_bytes(), Vec::new(), false);
        let outcome = run(&self.backend, &mut talk).await;
        (outcome, String::from_utf8_lossy(&talk.out).into_owned())
    }
}

/// A machine with no client on `PATH`; it remembers what was looked for.
#[derive(Default)]
struct Absent {
    asked: RefCell<Vec<String>>,
}

impl Program for Absent {
    fn run(&self, name: &str, _args: &[&str]) -> io::Result<Ran> {
        self.asked.borrow_mut().push(name.to_owned());
        Err(io::ErrorKind::NotFound.into())
    }
}

const LOGGED_IN: &str = "Logged in as ada@example.com. The MCP server picks the login up on its next call.\n";

#[tokio::test]
async fn a_right_code_saves_the_login_the_server_reads() -> Result<(), Trouble> {
    let rig = rig("success").await?;
    let (outcome, shown) = rig.setup("ada@example.com\n123456\n").await;
    outcome?;
    assert_eq!(
        shown,
        format!("Email: A code was sent to ada@example.com.\nCode: {LOGGED_IN}")
    );
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_wrong_code_asks_again_and_a_later_right_one_saves() -> Result<(), Trouble> {
    let rig = rig("again").await?;
    let (outcome, shown) = rig
        .setup("not an address\nada@example.com\n000000\n111111\n123456\n")
        .await;
    outcome?;
    assert_eq!(
        shown,
        format!(
            "Email: That does not look like an email address.\n\
             Email: A code was sent to ada@example.com.\n\
             Code: That code was not accepted. Try again.\n\
             Code: That code was not accepted. Try again.\n\
             Code: {LOGGED_IN}"
        )
    );
    assert!(rig.saved()?.is_some());
    Ok(())
}

#[tokio::test]
async fn three_wrong_codes_fail_and_save_nothing() -> Result<(), Trouble> {
    let rig = rig("refused").await?;
    let (outcome, shown) = rig
        .setup("ada@example.com\n000000\n111111\n222222\n123456\n")
        .await;
    assert!(matches!(outcome, Err(SetupError::Refused(Secret::Code))));
    assert!(
        shown.ends_with("Code: That code was not accepted.\n"),
        "{shown}"
    );
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn an_existing_login_stays_unless_the_answer_is_yes() -> Result<(), Trouble> {
    let rig = rig("existing").await?;
    rig.setup("ada@example.com\n123456\n").await.0?;
    let before = rig.saved()?;
    let (no, asked) = rig.setup("n\n").await;
    no?;
    assert!(asked.contains("Log in again? [y/N]: "), "{asked}");
    assert!(!asked.contains("Email"), "{asked}");
    assert_eq!(rig.saved()?, before);
    let (blank, left_blank) = rig.setup("\n").await;
    blank?;
    assert!(!left_blank.contains("Email"), "{left_blank}");
    let (yes, again) = rig.setup("y\nada@example.com\n123456\n").await;
    yes?;
    assert!(again.ends_with(LOGGED_IN), "{again}");
    Ok(())
}

#[tokio::test]
async fn the_end_of_input_stops_the_dialog() -> Result<(), Trouble> {
    let rig = rig("eof").await?;
    let (outcome, _) = rig.setup("ada@example.com\n").await;
    assert!(matches!(outcome, Err(SetupError::EndOfInput)));
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn no_connect_logs_in_and_leaves_the_clients_alone() -> Result<(), Trouble> {
    let rig = rig("no-connect").await?;
    let (outcome, shown, asked) = rig
        .steps("ada@example.com\n123456\n", Steps::LoginOnly)
        .await;
    outcome?;
    assert!(shown.ends_with(LOGGED_IN), "{shown}");
    assert!(asked.is_empty(), "{asked:?}");
    Ok(())
}

#[tokio::test]
async fn connect_only_skips_the_login() -> Result<(), Trouble> {
    let rig = rig("connect-only").await?;
    let (outcome, shown, asked) = rig.steps("", Steps::ConnectOnly).await;
    outcome?;
    assert!(
        shown.starts_with("Other clients: add the server by hand."),
        "{shown}"
    );
    assert_eq!(asked, ["claude", "codex"]);
    assert_eq!(rig.saved()?, None);
    Ok(())
}

#[tokio::test]
async fn both_steps_log_in_then_connect() -> Result<(), Trouble> {
    let rig = rig("both").await?;
    let (outcome, shown, asked) =
        rig.steps("ada@example.com\n123456\n", Steps::Both).await;
    outcome?;
    assert!(shown.contains(LOGGED_IN), "{shown}");
    assert!(shown.contains("\"mcpServers\""), "{shown}");
    assert_eq!(asked, ["claude", "codex"]);
    Ok(())
}
