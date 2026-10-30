//! The macOS Keychain through `/usr/bin/security`: no prompt after a rebuild.

use std::{
    ffi::OsString,
    io,
    io::{Read, Write as _},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::{
    error::{
        Error,
        Result,
        local_data::{KeychainError, LocalDataError},
    },
    storage::session_store::{
        ACCOUNT,
        Keychain,
        LoginRemoval,
        SERVICE,
        SessionStore,
        SessionStoreKind,
        security,
        security::{KeychainItem, SecurityExit},
    },
};

pub const WAIT: Duration = Duration::from_mins(2);

const POLL: Duration = Duration::from_millis(5);

const STDERR_ROOM: usize = 3 * security::MAX_LINE + 64;

/// The port: which `security` runs, on which keychain, how long it may take.
#[derive(Debug, Clone)]
pub struct SecurityTool {
    program: PathBuf,
    keychain: Option<String>,
    wait: Duration,
}

impl SecurityTool {
    /// `/usr/bin/security` on the default keychain and search list.
    #[must_use]
    pub fn system() -> Self {
        Self::with(PathBuf::from(security::PROGRAM), None, WAIT)
    }

    /// Another program or keychain file, and another wait.
    #[must_use]
    pub fn with(program: PathBuf, keychain: Option<String>, wait: Duration) -> Self {
        Self {
            program,
            keychain,
            wait,
        }
    }

    fn item(&self) -> KeychainItem<'_> {
        KeychainItem {
            service: SERVICE,
            account: ACCOUNT,
            keychain: self.keychain.as_deref(),
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the keychain is reached only through the macOS `security` program"
    )]
    fn run(
        &self,
        arguments: &[OsString],
        input: Option<&[u8]>,
    ) -> std::result::Result<(SecurityExit, Zeroizing<Vec<u8>>), KeychainError> {
        let failed = |source| KeychainError::Spawn {
            program: self.program.clone(),
            error: source,
        };
        let mut child = Command::new(&self.program)
            .args(arguments)
            .stdin(input.map_or_else(Stdio::null, |_| Stdio::piped()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(failed)?;
        // Read while it runs: a full pipe would block it until the timeout.
        let stdout = drain(child.stdout.take(), 0);
        let stderr = drain(child.stderr.take(), STDERR_ROOM);
        let settled =
            feed(&mut child, input).and_then(|()| settle(&mut child, self.wait));
        match settled {
            Ok(Some(status)) => collect(status, stdout, stderr).map_err(failed),
            Ok(None) => {
                stop(&mut child);
                Err(KeychainError::TimedOut(self.wait))
            }
            Err(source) => {
                stop(&mut child);
                Err(failed(source))
            }
        }
    }
}

type PipeDrain = thread::JoinHandle<io::Result<Zeroizing<Vec<u8>>>>;

fn drain<R: Read + Send + 'static>(pipe: Option<R>, room: usize) -> PipeDrain {
    thread::spawn(move || {
        let mut out = Zeroizing::new(Vec::with_capacity(room));
        pipe.map_or(Ok(0), |mut pipe| pipe.read_to_end(&mut out))
            .map(|_| out)
    })
}

fn drained(drain: PipeDrain) -> io::Result<Zeroizing<Vec<u8>>> {
    drain
        .join()
        .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::BrokenPipe)))
}

fn feed(child: &mut Child, input: Option<&[u8]>) -> io::Result<()> {
    child
        .stdin
        .take()
        .zip(input)
        .map_or(Ok(()), |(mut stdin, input)| stdin.write_all(input))
}

#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous poll of a child process with no runtime or clock: the pause blocks the thread"
)]
fn settle(child: &mut Child, wait: Duration) -> io::Result<Option<ExitStatus>> {
    let start = Instant::now();
    std::iter::repeat_with(|| child.try_wait())
        .find_map(|polled| match polled {
            Ok(None) if start.elapsed() < wait => {
                thread::sleep(POLL);
                None
            }
            Ok(None) => Some(Ok(None)),
            Ok(Some(status)) => Some(Ok(Some(status))),
            Err(err) => Some(Err(err)),
        })
        .unwrap_or(Ok(None))
}

fn stop(child: &mut Child) {
    if let Err(err) = child.kill() {
        tracing::debug!(%err, "keychain tool not killed");
    }
    if let Err(err) = child.wait() {
        tracing::debug!(%err, "keychain tool not reaped");
    }
}

fn collect(
    status: ExitStatus,
    stdout: PipeDrain,
    stderr: PipeDrain,
) -> io::Result<(SecurityExit, Zeroizing<Vec<u8>>)> {
    drained(stdout)?;
    let stderr = drained(stderr)?;
    let exit = security::classify_run(status.code(), &String::from_utf8_lossy(&stderr));
    Ok((exit, stderr))
}

impl SessionStore for SecurityTool {
    fn kind(&self) -> SessionStoreKind {
        SessionStoreKind::Keychain
    }

    fn read(&self) -> Result<Option<SecretString>> {
        let (exit, stderr) = self.run(&security::find_arguments(&self.item()), None)?;
        match exit {
            SecurityExit::Done => security::decode_password(&stderr)
                .map(|mut value| Some(SecretString::from(std::mem::take(&mut *value))))
                .ok_or(Error::LocalData(LocalDataError::SavedLoginCorrupt)),
            SecurityExit::Missing => Ok(None),
            SecurityExit::Error(err) => Err(err.into()),
        }
    }

    fn write(&self, stored: &SecretString) -> Result<()> {
        let line = security::add_line(&self.item(), stored.expose_secret().as_bytes())?;
        let (exit, _) = self.run(&[OsString::from("-i")], Some(&line))?;
        exit.required().map_err(Error::from)
    }

    fn remove(&self) -> Result<LoginRemoval> {
        let (exit, _) = self.run(&security::delete_arguments(&self.item()), None)?;
        match exit {
            SecurityExit::Done => Ok(LoginRemoval::Removed),
            SecurityExit::Missing => Ok(LoginRemoval::Absent),
            SecurityExit::Error(err) => Err(err.into()),
        }
    }
}

impl SessionStore for Keychain {
    fn kind(&self) -> SessionStoreKind {
        SessionStoreKind::Keychain
    }

    fn read(&self) -> Result<Option<SecretString>> {
        SecurityTool::system().read()
    }

    fn write(&self, stored: &SecretString) -> Result<()> {
        SecurityTool::system().write(stored)
    }

    fn remove(&self) -> Result<LoginRemoval> {
        SecurityTool::system().remove()
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use std::{
        fs,
        io,
        os::unix::fs::PermissionsExt as _,
        path::{Path, PathBuf},
        process::{Command, Output},
        time::{Duration, Instant},
    };

    use secrecy::{ExposeSecret, SecretString};

    use crate::{
        error::{
            Error,
            Result,
            local_data::{KeychainError, LocalDataError},
        },
        storage::session_store::{
            LoginRemoval,
            SessionStore,
            macos::{SecurityTool, WAIT},
            security,
        },
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(test: &str) -> io::Result<Self> {
            let dir = std::env::temp_dir().join(format!(
                "holst-core-security-{test}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }

        fn script(&self, name: &str, body: &str) -> io::Result<PathBuf> {
            let path = self.0.join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
            Ok(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(err) = fs::remove_dir_all(&self.0) {
                tracing::debug!(%err, "test directory not removed");
            }
        }
    }

    fn recorder(dir: &TestDirectory, answer: &str) -> io::Result<PathBuf> {
        dir.script(
            "security",
            &format!(
                "dir=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" >> \"$dir/argv\"\ncat >> \
                 \"$dir/stdin\"\n{answer}"
            ),
        )
    }

    fn recorded(dir: &TestDirectory, file: &str) -> io::Result<String> {
        fs::read_to_string(dir.0.join(file)).or_else(|err| {
            (err.kind() == io::ErrorKind::NotFound)
                .then(String::new)
                .ok_or(err)
        })
    }

    fn hex(text: &str) -> String {
        Some(Vec::new())
            .map(|mut out| {
                security::hex_into(&mut out, text.as_bytes());
                out
            })
            .and_then(|out| String::from_utf8(out).ok())
            .unwrap_or_default()
    }

    #[test]
    fn secret_goes_only_to_stdin_and_only_as_hex() -> Result<()> {
        let dir = TestDirectory::new("argv")?;
        let fake = recorder(&dir, "exit 0")?;
        let keychain = dir.0.join("fake.keychain").display().to_string();
        let store = SecurityTool::with(fake, Some(keychain.clone()), WAIT);
        let secret = format!(
            "{{\"token\":\"s3cr3t-{}\",\"cookies\":\"a=b\"}}",
            uuid::Uuid::new_v4()
        );
        store.write(&SecretString::from(secret.clone()))?;
        let (argv, stdin) = (recorded(&dir, "argv")?, recorded(&dir, "stdin")?);
        assert_eq!(argv, "-i\n");
        assert!(
            !stdin.contains(&secret) && !stdin.contains("s3cr3t"),
            "{stdin}"
        );
        assert_eq!(
            stdin,
            format!(
                "add-generic-password -U -s \"holst-rs\" -a \"session-token\" -X {} \"{keychain}\"\n",
                hex(&secret)
            )
        );
        Ok(())
    }

    #[test]
    fn read_and_remove_name_only_the_item() -> Result<()> {
        let dir = TestDirectory::new("read")?;
        let answer = "printf '%s\\n' 'password: 0x7B22746F6B656E223A2274227D  \"{\"token\":\"t\"}\"' >&2";
        let fake = recorder(&dir, answer)?;
        let store = SecurityTool::with(fake, None, WAIT);
        let read = store.read()?.map(|value| value.expose_secret().to_owned());
        assert_eq!(read.as_deref(), Some("{\"token\":\"t\"}"));
        assert_eq!(store.remove()?, LoginRemoval::Removed);
        assert_eq!(
            recorded(&dir, "argv")?,
            "find-generic-password\n-s\nholst-rs\n-a\nsession-token\n-g\n\
             delete-generic-password\n-s\nholst-rs\n-a\nsession-token\n"
        );
        assert_eq!(recorded(&dir, "stdin")?, "");
        Ok(())
    }

    #[test]
    fn exit_codes_become_typed_errors() -> Result<()> {
        let dir = TestDirectory::new("codes")?;
        let store = |code: u8| {
            dir.script(&format!("security-{code}"), &format!("exit {code}"))
                .map(|fake| SecurityTool::with(fake, None, WAIT))
        };
        assert!(store(44)?.read()?.is_none());
        assert_eq!(store(44)?.remove()?, LoginRemoval::Absent);
        let failure = |code| store(code).map(|s| s.read().map(|_| ()));
        assert!(matches!(
            failure(128)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Cancelled
            )))
        ));
        assert!(matches!(
            failure(36)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Locked
            )))
        ));
        assert!(matches!(
            failure(51)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Denied
            )))
        ));
        assert!(matches!(
            failure(2)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Exited { status: 2 }
            )))
        ));
        Ok(())
    }

    #[test]
    fn removal_that_fails_is_an_error_not_absent() -> Result<()> {
        let dir = TestDirectory::new("remove")?;
        let removal = |code: u8| {
            dir.script(&format!("security-{code}"), &format!("exit {code}"))
                .map(|fake| SecurityTool::with(fake, None, WAIT).remove())
        };
        assert!(matches!(
            removal(36)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Locked
            )))
        ));
        assert!(matches!(
            removal(51)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Denied
            )))
        ));
        assert!(matches!(
            removal(128)?,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Cancelled
            )))
        ));
        let hung = dir.script("security-hung", "exec sleep 30")?;
        let timed_out =
            SecurityTool::with(hung, None, Duration::from_millis(200)).remove();
        assert!(
            matches!(
                timed_out,
                Err(Error::LocalData(LocalDataError::Keychain(
                    KeychainError::TimedOut(_)
                )))
            ),
            "{timed_out:?}"
        );
        Ok(())
    }

    #[test]
    fn large_output_does_not_block_security() -> Result<()> {
        let dir = TestDirectory::new("large")?;
        let big = 300_000;
        let answer = format!(
            "head -c {big} /dev/zero | tr '\\0' 'x'\nprintf 'password: \"' >&2\nhead -c {big} \
             /dev/zero | tr '\\0' 'a' >&2\nprintf '\"\\n' >&2"
        );
        let fake = dir.script("security", &answer)?;
        let started = Instant::now();
        let read = SecurityTool::with(fake, None, Duration::from_secs(20)).read()?;
        assert_eq!(read.map(|value| value.expose_secret().len()), Some(big));
        assert!(started.elapsed() < Duration::from_secs(10));
        Ok(())
    }

    #[test]
    fn a_hung_security_is_stopped() -> Result<()> {
        let dir = TestDirectory::new("hang")?;
        let fake = dir.script("security", "exec sleep 30")?;
        let wait = Duration::from_millis(200);
        let started = Instant::now();
        let read = SecurityTool::with(fake, None, wait).read();
        assert!(
            matches!(
                read,
                Err(Error::LocalData(LocalDataError::Keychain(
                    KeychainError::TimedOut(_)
                )))
            ),
            "{read:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        Ok(())
    }

    #[test]
    fn a_missing_program_is_reported() {
        let store =
            SecurityTool::with(PathBuf::from("/nonexistent/security"), None, WAIT);
        assert!(matches!(
            store.read(),
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::Spawn { .. }
            )))
        ));
    }

    fn tool(arguments: &[&str]) -> io::Result<Output> {
        Command::new(security::PROGRAM).args(arguments).output()
    }

    fn user_keychains() -> io::Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            tool(&["list-keychains", "-d", "user"])?.stdout,
            tool(&["default-keychain", "-d", "user"])?.stdout,
        ))
    }

    struct TempKeychain {
        path: String,
        _dir: TestDirectory,
    }

    impl TempKeychain {
        fn new() -> io::Result<Self> {
            let dir = TestDirectory::new("keychain")?;
            let path = dir.0.join("test.keychain").display().to_string();
            let password = uuid::Uuid::new_v4().simple().to_string();
            let created = tool(&["create-keychain", "-p", &password, &path])?;
            assert!(created.status.success(), "{created:?}");
            let unlocked = tool(&["unlock-keychain", "-p", &password, &path])?;
            assert!(unlocked.status.success(), "{unlocked:?}");
            Ok(Self { path, _dir: dir })
        }

        fn store(&self) -> SecurityTool {
            SecurityTool::with(
                PathBuf::from(security::PROGRAM),
                Some(self.path.clone()),
                WAIT,
            )
        }
    }

    impl Drop for TempKeychain {
        fn drop(&mut self) {
            if let Err(err) = tool(&["delete-keychain", &self.path]) {
                tracing::debug!(%err, "test keychain not deleted");
            }
        }
    }

    fn value(store: &SecurityTool) -> Result<Option<String>> {
        Ok(store.read()?.map(|value| value.expose_secret().to_owned()))
    }

    #[test]
    fn round_trip_through_a_temporary_keychain() -> Result<()> {
        let before = user_keychains()?;
        let keychain = TempKeychain::new()?;
        let store = keychain.store();
        assert!(Path::new(&keychain.path).exists());
        assert_eq!(user_keychains()?, before);
        assert_eq!(value(&store)?, None);
        let first =
            "{\"token\":\"0b2f7c8e-1d3a-4c5b-9e6f-7a8b9c0d1e2f\",\"cookies\":\"s=1\"}";
        store.write(&SecretString::from(first))?;
        assert_eq!(value(&store)?.as_deref(), Some(first));
        let second = "{\"token\":\"t\u{e9}\\\"\",\"cookies\":\"a=b; c=d\\n\"}\t";
        store.write(&SecretString::from(second))?;
        assert_eq!(value(&store)?.as_deref(), Some(second));
        // Printable hex digits come back as they are, not decoded as hex.
        ["aaaa", "0b2f7c8e1d3a4c5b9e6f7a8b9c0d1e2f", "4142"]
            .into_iter()
            .try_for_each(|bare| {
                store.write(&SecretString::from(bare))?;
                assert_eq!(value(&store)?.as_deref(), Some(bare));
                Ok::<_, Error>(())
            })?;
        let room = security::room(&store.item())?;
        let longest = "x".repeat(room);
        store.write(&SecretString::from(longest.clone()))?;
        assert_eq!(value(&store)?, Some(longest));
        let too_long = store.write(&SecretString::from("x".repeat(room + 1)));
        assert!(matches!(
            too_long,
            Err(Error::LocalData(LocalDataError::Keychain(
                KeychainError::TooLong { .. }
            )))
        ));
        assert_eq!(store.remove()?, LoginRemoval::Removed);
        assert_eq!(value(&store)?, None);
        assert_eq!(store.remove()?, LoginRemoval::Absent);
        drop(keychain);
        assert_eq!(user_keychains()?, before);
        Ok(())
    }
}
