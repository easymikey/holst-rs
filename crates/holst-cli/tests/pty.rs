//! The login form in a pseudo-terminal; the secret never shows.

#![cfg(unix)]
#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests build and read real files in temp directories"
)]

mod support;

use std::{
    cell::RefCell,
    io::Read as _,
    net::TcpListener,
    process::Command,
    thread,
    time::Duration,
};

use expectrl::{
    Eof,
    Expect as _,
    Session,
    process::unix::WaitStatus,
    session::OsSession,
};

#[derive(Debug, thiserror::Error)]
enum TestError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Pty(#[from] expectrl::Error),
}

type TestResult = Result<(), TestError>;

fn empty_home() -> std::io::Result<std::path::PathBuf> {
    static RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("holst-pty-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).map(|()| dir)
}

struct Form {
    session: RefCell<OsSession>,
    seen: RefCell<String>,
}

/// Where and how the shell wrapper runs `holst auth login`.
struct ShellLogin<'a> {
    base: &'a str,
    variables: &'a str,
    flags: &'a str,
    home: &'a std::path::Path,
    after: &'a str,
}

/// Builds a command with the test environment around a fake server and home.
fn test_command(
    program: &str,
    arguments: &[&str],
    base: &str,
    home: &std::path::Path,
) -> Command {
    // imperative: `Command` is a `&mut` builder.
    let mut command = support::isolated(program, home);
    command
        .args(arguments)
        .env("HOLST_BASE_URL", base)
        .env("TERM", "xterm-256color")
        .env("NO_COLOR", "1");
    command
}

impl ShellLogin<'_> {
    fn start(&self) -> Result<Form, expectrl::Error> {
        let script = format!(
            "trap true INT; stty echo; {env} '{holst}' auth login {flags}; echo \"EXIT:$?\"; {after}",
            env = self.variables,
            flags = self.flags,
            after = self.after,
            holst = env!("CARGO_BIN_EXE_holst"),
        );
        Form::spawn(test_command("sh", &["-c", &script], self.base, self.home))
    }
}

impl Form {
    fn spawn(command: Command) -> Result<Self, expectrl::Error> {
        let session = Session::spawn(command).map(|mut session| {
            session.set_expect_timeout(Some(Duration::from_secs(15)));
            session
        })?;
        Ok(Self {
            session: RefCell::new(session),
            seen: RefCell::new(String::new()),
        })
    }

    fn start(base: &str) -> Result<Self, expectrl::Error> {
        Self::spawn(test_command(
            env!("CARGO_BIN_EXE_holst"),
            &["auth", "login", "--no-save"],
            base,
            &empty_home()?,
        ))
    }

    fn in_shell(base: &str, env: &str, after: &str) -> Result<Self, expectrl::Error> {
        ShellLogin {
            base,
            variables: env,
            flags: "--no-save",
            home: &empty_home()?,
            after,
        }
        .start()
    }

    fn wait_for_holst(&self) -> std::io::Result<()> {
        let shell = self.session.borrow().get_process().pid().to_string();
        std::iter::repeat_with(|| {
            thread::sleep(Duration::from_millis(5));
            Command::new("pgrep")
                .args(["-P", &shell, "-x", "holst"])
                .output()
                .map(|out| !out.stdout.is_empty())
        })
        .take(400)
        .find(|started| !matches!(started, Ok(false)))
        .unwrap_or(Ok(false))
        .map(drop)
    }

    fn signal_holst(&self, signal: &str) -> std::io::Result<()> {
        let shell = self.session.borrow().get_process().pid().to_string();
        Command::new("pkill")
            .args([&format!("-{signal}"), "-P", &shell])
            .status()
            .map(drop)
    }

    fn wait_for(&self, text: &str) -> Result<(), expectrl::Error> {
        let found = self.session.borrow_mut().expect(text)?;
        self.record(&found);
        Ok(())
    }

    fn press(&self, keys: &str) -> Result<(), expectrl::Error> {
        self.session.borrow_mut().send(keys)
    }

    fn finish(self) -> Result<(Option<i32>, String), TestError> {
        self.wait_for_end()?;
        let status = self
            .session
            .borrow()
            .get_process()
            .wait()
            .map_err(std::io::Error::from)?;
        let code = match status {
            WaitStatus::Exited(_, code) => Some(code),
            WaitStatus::Signaled(..)
            | WaitStatus::Stopped(..)
            | WaitStatus::Continued(_)
            | WaitStatus::StillAlive => None,
            #[cfg(any(target_os = "linux", target_os = "android"))]
            WaitStatus::PtraceEvent(..) | WaitStatus::PtraceSyscall(_) => None,
        };
        Ok((code, self.seen.into_inner()))
    }

    fn wait_for_end(&self) -> Result<(), expectrl::Error> {
        let found = self.session.borrow_mut().expect(Eof)?;
        self.record(&found);
        Ok(())
    }

    fn record(&self, found: &expectrl::Captures) {
        self.seen.borrow_mut().extend(
            std::iter::once(found.before())
                .chain(found.matches())
                .map(String::from_utf8_lossy),
        );
    }
}

fn printed(raw: &str) -> String {
    raw.chars()
        .scan(false, |in_escape, c| {
            let (next, keep) = match (*in_escape, c) {
                (false, '\x1b') => (true, None),
                (false, c) => (false, Some(c)),
                (true, 'A'..='Z' | 'a'..='z') => (false, None),
                (true, _) => (true, None),
            };
            *in_escape = next;
            Some(keep)
        })
        .flatten()
        .collect()
}

#[test]
fn typos_backspace_and_a_retry_then_success() -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = Form::start(&base)?;
    form.wait_for("◆  Email")?;
    form.press("bad\r")?;
    form.wait_for("enter an address like you@example.com")?;
    form.press("mock@example.comXY\x7f\x7f\r")?;
    form.wait_for("◆  Code")?;
    form.press(&format!("{}\r", holst_core::fake::LOGIN_CODE))?;
    form.wait_for("└  Done")?;
    let (code, seen) = form.finish()?;
    let shown = printed(&seen);
    assert_eq!(code, Some(0), "{shown}");
    assert!(shown.contains("◇  Email"), "{shown}");
    assert!(shown.contains("mock@example.com"), "{shown}");
    assert!(!shown.contains("mock@example.comXY\n"), "{shown}");
    assert!(
        !shown.contains(holst_core::fake::LOGIN_CODE),
        "the code was on screen:\n{shown}"
    );
    assert!(shown.contains("▪▪▪▪▪▪"), "{shown}");
    Ok(())
}

#[test]
fn arrows_edit_inside_the_answer() -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = Form::start(&base)?;
    form.wait_for("◆  Email")?;
    form.press("mock@exmple.com")?;
    form.press(&"\x1b[D".repeat(8))?;
    form.press("a\r")?;
    form.wait_for("◆  Code")?;
    form.press("12345x6\x1b[D\x7f\r")?;
    form.wait_for("└  Done")?;
    let (code, seen) = form.finish()?;
    let shown = printed(&seen);
    assert_eq!(code, Some(0), "{shown}");
    assert!(shown.contains("│  mock@example.com"), "{shown}");
    assert!(shown.contains("◇  Code\r\n│  ▪▪▪▪▪▪\r\n"), "{shown}");
    assert!(!shown.contains("12345"), "the code was on screen:\n{shown}");
    Ok(())
}

#[test]
fn ctrl_c_cancels_with_a_message() -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = Form::start(&base)?;
    form.wait_for("◆  Email")?;
    form.press("mock@example.com\r")?;
    form.wait_for("◆  Code")?;
    form.press("12")?;
    form.wait_for("▪▪")?;
    form.press("\x03")?;
    let (code, seen) = form.finish()?;
    let shown = printed(&seen);
    assert_eq!(code, Some(130), "{shown}");
    // The question stays, marked `■`, with the given-up mask under it.
    let kept = shown.split_once("■  Code").map(|(_, rest)| rest);
    assert!(kept.is_some_and(|rest| rest.contains("│  ▪▪")), "{shown}");
    assert!(
        kept.is_some_and(|rest| rest.contains("└  login cancelled")),
        "{shown}"
    );
    Ok(())
}

const ECHO_CHECK: &str =
    r#"case "$(stty -a)" in *" -echo "*) echo ECHO:off;; *) echo ECHO:on;; esac"#;

#[rstest::rstest]
#[case::on_the_address(&[])]
#[case::on_the_code(&[("mock@example.com\r", "◆  Code")])]
fn ctrl_c_in_a_blind_form_ends_it_with_the_echo_back(
    #[case] before: &[(&str, &str)],
) -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = Form::in_shell(&base, "TERM=dumb", ECHO_CHECK)?;
    form.wait_for("◆  Email")?;
    before.iter().try_for_each(|(keys, next)| {
        form.press(keys).and_then(|()| form.wait_for(next))
    })?;
    form.press("\x03")?;
    let (_, seen) = form.finish()?;
    let shown = printed(&seen);
    assert!(shown.contains("└  login cancelled"), "{shown}");
    assert!(shown.contains("EXIT:130"), "{shown}");
    assert!(shown.contains("ECHO:on"), "{shown}");
    Ok(())
}

#[rstest::rstest]
#[case::term("TERM", "EXIT:143")]
#[case::hangup("HUP", "EXIT:129")]
fn signals_on_the_code_leave_the_terminal_as_it_was(
    #[case] signal: &str,
    #[case] exit: &str,
) -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = Form::in_shell(&base, "", ECHO_CHECK)?;
    form.wait_for("◆  Email")?;
    form.press("mock@example.com\r")?;
    form.wait_for("◆  Code")?;
    form.press("12")?;
    form.wait_for("▪▪")?;
    form.signal_holst(signal)?;
    let (_, seen) = form.finish()?;
    let shown = printed(&seen);
    assert!(shown.contains(exit), "{shown}");
    assert!(shown.contains("ECHO:on"), "{shown}");
    Ok(())
}

#[test]
fn rest_of_a_paste_does_not_reach_the_shell() -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let after = r#"stty -icanon min 0 time 10; echo "LEFT:[$(head -c 64)]""#;
    let form = Form::in_shell(&base, "", after)?;
    form.wait_for("◆  Email")?;
    form.press("mock@example.com\r")?;
    form.wait_for("◆  Code")?;
    form.press(&format!("{}\necho LEAK\n", holst_core::fake::LOGIN_CODE))?;
    let (_, seen) = form.finish()?;
    let shown = printed(&seen);
    assert!(shown.contains("│  Fake User"), "{shown}");
    assert!(shown.contains("EXIT:0"), "{shown}");
    assert!(shown.contains("LEFT:[]"), "{shown}");
    assert!(!shown.contains("LEAK"), "{shown}");
    Ok(())
}

fn screen(raw: &str) -> Vec<String> {
    raw.split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r');
            let after_wipe = line.rsplit("\x1b[2K").next().unwrap_or_default();
            printed(after_wipe.rsplit('\r').next().unwrap_or_default())
        })
        .collect()
}

fn assert_cancelled_cleanly(seen: &str) {
    let failed = failed_check(seen);
    assert!(failed.is_none(), "{failed:?}\nRAW: {seen:?}");
}

fn failed_check(seen: &str) -> Option<String> {
    let lines = screen(seen);
    let shown = lines.join("\n");
    let checks = [
        ("no ^C on screen", !shown.contains("^C")),
        (
            "┌ in the first column",
            lines
                .iter()
                .any(|line| line.starts_with("┌   holst auth login")),
        ),
        ("└  login cancelled", shown.contains("└  login cancelled")),
        ("exit code 130", shown.contains("EXIT:130")),
        ("echo back on", shown.contains("ECHO:on")),
    ];
    checks
        .iter()
        .find(|(_, ok)| !ok)
        .map(|(what, _)| format!("{what} failed:\n{shown}"))
}

fn hanging_store() -> std::io::Result<std::path::PathBuf> {
    let home = empty_home()?;
    Command::new("mkfifo")
        .arg(home.join("session"))
        .status()
        .map(|_| home)
}

#[rstest::rstest]
#[case::once(1)]
#[case::thrice(3)]
fn ctrl_c_while_the_saved_login_is_read(#[case] times: usize) -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = ShellLogin {
        base: &base,
        variables: "",
        flags: "",
        home: &hanging_store()?,
        after: ECHO_CHECK,
    }
    .start()?;
    form.wait_for("Reading the saved login")?;
    // All at once: later ones would reach the shell after `holst` is gone.
    form.press(&"\x03".repeat(times))?;
    let (_, seen) = form.finish()?;
    assert_cancelled_cleanly(&seen);
    Ok(())
}

#[test]
fn ctrl_c_right_after_the_start() -> TestResult {
    let base = support::start()?.base_url().to_owned();
    let form = ShellLogin {
        base: &base,
        variables: "",
        flags: "",
        home: &hanging_store()?,
        after: ECHO_CHECK,
    }
    .start()?;
    form.wait_for_holst()?;
    thread::sleep(Duration::from_millis(20));
    form.press("\x03\x03\x03")?;
    let (_, seen) = form.finish()?;
    let shown = screen(&seen).join("\n");
    let untouched =
        !shown.contains('┌') && shown.contains("EXIT:130") && shown.contains("ECHO:on");
    let failed = failed_check(&seen);
    assert!(failed.is_none() || untouched, "{failed:?}\nRAW: {seen:?}");
    Ok(())
}

fn stuck_on_sending() -> std::io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}", listener.local_addr()?);
    thread::spawn(move || {
        listener.incoming().flatten().for_each(|mut stream| {
            let head = Some([0_u8; 4096])
                .map_or_else(String::new, |mut request| {
                    let read = stream.read(&mut request).unwrap_or(0);
                    String::from_utf8_lossy(request.get(..read).unwrap_or_default())
                        .into_owned()
                });
            let options = r#"{"hasAccount":true,"viaTemporaryPassword":true}"#;
            head.contains("get_public_options")
                .then(|| {
                    use std::io::Write as _;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{options}",
                        options.len()
                    )
                })
                .transpose()
                .map(drop)
                .unwrap_or_default(); // a failed stub write only makes the client test fail
            // The code request stays unanswered: keep its connection open.
            thread::spawn(move || {
                thread::sleep(Duration::from_mins(1));
                drop(stream);
            });
        });
    });
    Ok(base)
}

#[test]
fn ctrl_c_while_the_code_is_sent() -> TestResult {
    let base = stuck_on_sending()?;
    let form = Form::in_shell(&base, "", ECHO_CHECK)?;
    form.wait_for("◆  Email")?;
    form.press("mock@example.com\r")?;
    form.wait_for("Sending the code")?;
    form.press("\x03")?;
    let (_, seen) = form.finish()?;
    assert_cancelled_cleanly(&seen);
    assert!(!printed(&seen).contains("error: interrupted"));
    Ok(())
}
