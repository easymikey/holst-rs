//! Leftover files in a home, and the shutdown of a running command.

use std::process::Stdio;

use crate::{
    fake,
    support::{REVOKED, TestResult, holst_at, home, stderr},
};

#[cfg(unix)]
#[test]
fn a_test_command_sees_only_its_own_holst_variables() -> TestResult {
    let dir = home(None)?;
    let exported = [
        ("PATH".into(), std::env::var_os("PATH").unwrap_or_default()),
        ("HOLST_MCP_RAW_WRITE".into(), "1".into()),
        ("HOLST_CACHE_MAX".into(), "1".into()),
    ];
    let out = fake::isolated_from("env", &dir, exported).output()?;
    let printed = String::from_utf8_lossy(&out.stdout);
    let mut names: Vec<&str> = printed
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .filter(|name| name.starts_with("HOLST_"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["HOLST_CACHE_DIR", "HOLST_HOME"]);
    Ok(())
}

#[test]
fn a_cache_directory_exported_by_the_developer_is_left_alone() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let dir = home(Some(r#"{"cookies":"x"}"#))?;
    let developers = dir.with_extension("developers");
    let marker = developers.join("v1").join("marker");
    std::fs::create_dir_all(marker.parent().unwrap_or(&developers))?;
    std::fs::write(&marker, "")?;
    let exported = [
        ("PATH", std::env::var_os("PATH")),
        ("LLVM_PROFILE_FILE", std::env::var_os("LLVM_PROFILE_FILE")),
    ]
    .into_iter()
    .filter_map(|(name, value)| Some((name.into(), value?)))
    .chain([("HOLST_CACHE_DIR".into(), developers.into_os_string())]);
    let out = fake::isolated_from(env!("CARGO_BIN_EXE_holst"), &dir, exported)
        .args(["auth", "logout"])
        .env("HOLST_BASE_URL", &base)
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(marker.exists(), "logout wiped the developer's cache");
    Ok(())
}

#[test]
fn storage_lists_and_cleans_only_its_own_leftovers() -> TestResult {
    let root = home(None)?;
    let fake_home = root.join("home");
    let config = fake_home.join(".config/holst");
    std::fs::create_dir_all(config.join("browser/Default"))?;
    std::fs::write(config.join("browser/Default/Cookies"), [0u8; 2048])?;
    std::fs::write(config.join(".token.12-0.tmp"), "x")?;
    let dir = root.join("hh");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("session"), REVOKED)?;
    std::fs::write(dir.join(".session.7-1.tmp"), "x")?;
    std::fs::write(dir.join("notes.txt"), "not holst's")?;

    let listed = holst_at(&fake_home, &dir, &["storage", "ls"])?;
    let text = String::from_utf8_lossy(&listed.stdout);
    assert_eq!(listed.status.code(), Some(0), "{}", stderr(&listed));
    assert!(
        text.contains("browser") && text.contains(".session.7-1.tmp"),
        "{text}"
    );
    assert!(!text.contains("notes.txt"), "{text}");

    let dry = holst_at(&fake_home, &dir, &["storage", "clean", "--dry-run"])?;
    assert!(String::from_utf8_lossy(&dry.stdout).contains("would remove"));
    assert!(
        config.join("browser").exists(),
        "a dry run removed something"
    );

    let logout = holst_at(&fake_home, &dir, &["auth", "logout", "--local-only"])?;
    assert!(
        stderr(&logout).contains("holst storage clean"),
        "{}",
        stderr(&logout)
    );
    std::fs::write(dir.join("session"), REVOKED)?;

    let cleaned = holst_at(&fake_home, &dir, &["storage", "clean"])?;
    assert_eq!(cleaned.status.code(), Some(0), "{}", stderr(&cleaned));
    assert!(
        !config.join("browser").exists() && !config.join(".token.12-0.tmp").exists()
    );
    assert!(!dir.join(".session.7-1.tmp").exists());
    assert!(dir.join("session").exists(), "clean removed the login");
    assert!(dir.join("notes.txt").exists());

    let held = holst_at(&fake_home, &dir, &["storage", "clean", "--all"])?;
    assert_eq!(held.status.code(), Some(77), "{}", stderr(&held));
    assert!(
        dir.join("session").exists(),
        "--all without -y removed the login"
    );

    let all = holst_at(&fake_home, &dir, &["storage", "clean", "--all", "-y"])?;
    assert_eq!(all.status.code(), Some(0), "{}", stderr(&all));
    assert!(!dir.join("session").exists());
    assert!(dir.join("notes.txt").exists());
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

/// SIGTERM sent to the built `holst` while a reply is on the wire.
#[cfg(unix)]
mod terminated {
    use std::{
        fs::File,
        io::{self, BufRead as _, BufReader, Write as _},
        net::{TcpListener, TcpStream},
        path::PathBuf,
        process::{Command, ExitStatus, Stdio},
        sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
        thread,
        time::Duration,
    };

    use crate::{
        fake,
        support::{REVOKED, TestResult, home, saved_for},
    };

    /// The first reply of the run: the category list, with a rotated cookie.
    const CATEGORIES: &str = r#"{"categories":[{"id":"61971ca1-e031-4995-970a-aa764f7a91c1","slug":"strategic-planning","name":"Strategic planning","name_ru":"Планирование","icon":"planning","standard":true,"order_index":0,"count_templates":13,"created_at":"2026-07-14T18:46:40.616435Z","updated_at":"2026-07-14T18:46:40.616435Z"}]}"#;
    /// Longer than the 100 ms `main` gives the command's own stop.
    const PAST_THE_GRACE: Duration = Duration::from_millis(400);
    /// What the child needs to take in the reply it was just sent.
    const TO_READ_HEADERS: Duration = Duration::from_millis(200);
    const ENOUGH: Duration = Duration::from_secs(10);

    struct Rotating {
        pid: u32,
        exited: Receiver<io::Result<ExitStatus>>,
        dir: PathBuf,
        // Held so that the save of the rotated login has to wait.
        lock: File,
    }

    fn read_head(stream: &TcpStream) {
        BufReader::new(stream)
            .lines()
            .map_while(Result::ok)
            .take_while(|line| !line.is_empty())
            .for_each(drop);
    }

    fn reply(mut stream: TcpStream) -> io::Result<()> {
        read_head(&stream);
        let head = format!(
            "HTTP/1.1 200 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nset-cookie: sessionToken=rotated; Path=/; HttpOnly\r\nconnection: close\r\n\r\n{CATEGORIES}",
            CATEGORIES.len()
        );
        stream.write_all(head.as_bytes())
    }

    fn stall(mut stream: TcpStream, sent: &Sender<()>) -> io::Result<()> {
        read_head(&stream);
        sent.send(()).map_err(io::Error::other)?;
        // The client hangs up when it is stopped.
        io::copy(&mut stream, &mut io::sink()).map(drop)
    }

    /// Answers the first request and never the second.
    fn serve_stalled() -> io::Result<(String, Receiver<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let (sender, receiver) = mpsc::channel();
        drop(thread::spawn(move || {
            listener.incoming().flatten().take(2).enumerate().for_each(
                |(index, stream)| match index {
                    0 => reply(stream).unwrap_or_default(),
                    _ => stall(stream, &sender).unwrap_or_default(),
                },
            );
        }));
        Ok((base, receiver))
    }

    fn start_rotating() -> io::Result<Rotating> {
        let (base, sent) = serve_stalled()?;
        let dir = home(Some(&saved_for(&base, REVOKED)))?;
        let lock = File::create(dir.join("LOCK"))?;
        lock.lock()?;
        let mut child = fake::isolated(env!("CARGO_BIN_EXE_holst"), &dir)
            .args([
                "template",
                "ls",
                "--view",
                "standard",
                "--category",
                "Strategic planning",
            ])
            .env("HOLST_BASE_URL", base)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let pid = child.id();
        let (done, exited) = mpsc::channel();
        drop(thread::spawn(move || done.send(child.wait())));
        sent.recv_timeout(ENOUGH).map_err(io::Error::other)?;
        thread::sleep(TO_READ_HEADERS);
        Ok(Rotating {
            pid,
            exited,
            dir,
            lock,
        })
    }

    impl Rotating {
        fn terminate(&self) -> io::Result<()> {
            Command::new("kill")
                .args(["-TERM", &self.pid.to_string()])
                .status()
                .map(drop)
        }

        fn still_running(&self) -> bool {
            matches!(
                self.exited.recv_timeout(PAST_THE_GRACE),
                Err(RecvTimeoutError::Timeout)
            )
        }

        fn exit_code_within(&self, wait: Duration) -> io::Result<Option<i32>> {
            self.exited
                .recv_timeout(wait)
                .map_err(io::Error::other)?
                .map(|status| status.code())
        }
    }

    #[test]
    fn sigterm_waits_for_the_save_of_the_rotated_login() -> TestResult {
        let run = start_rotating()?;
        run.terminate()?;
        assert!(
            run.still_running(),
            "the process left before the save ended"
        );
        run.lock.unlock()?;
        assert_eq!(run.exit_code_within(ENOUGH)?, Some(143));
        let saved = std::fs::read_to_string(run.dir.join("session"))?;
        assert!(saved.contains("rotated"), "{saved}");
        Ok(())
    }

    #[test]
    fn a_second_sigterm_ends_the_process_at_once() -> TestResult {
        let run = start_rotating()?;
        run.terminate()?;
        assert!(
            run.still_running(),
            "the process left before the save ended"
        );
        run.terminate()?;
        // The save would wait for the lock for two seconds.
        assert_eq!(run.exit_code_within(Duration::from_secs(1))?, Some(143));
        Ok(())
    }
}
