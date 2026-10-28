//! The source installer and `scripts/cross-check.sh`, faked tools on `PATH`.

#![cfg(unix)]
#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Output},
};

type TestResult = std::io::Result<()>;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(test: &str) -> std::io::Result<Self> {
        let dir = std::env::temp_dir()
            .join(format!("holst-scripts-{test}-{}", std::process::id()));
        fs::remove_dir_all(&dir).unwrap_or_default();
        fs::create_dir_all(dir.join("bin"))?;
        Ok(Self(dir))
    }

    fn tool(&self, name: &str, body: &str) -> TestResult {
        let path = self.0.join("bin").join(name);
        let log = self.0.join("calls");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"{name} $*\" >> '{}'\n{body}\n",
                log.display()
            ),
        )?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.0.join("calls")).unwrap_or_default()
    }

    fn run(
        &self,
        script: &Path,
        script_arguments: &[&str],
        env: &[(&str, &str)],
    ) -> std::io::Result<Output> {
        let path = format!(
            "{}:{}",
            self.0.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new("sh")
            .arg(script)
            .args(script_arguments)
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .env_remove("HOLST_SIGN_IDENTITY")
            .env_remove("HOLST_XC_CHECKS")
            .env_remove("HOLST_XC_INSTALL_SH")
            .envs(env.iter().copied())
            .output()
    }
}

// GUARD: removes the scratch directory of the test.
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap_or_default();
    }
}

fn fake_mac(scratch: &TestDirectory) -> TestResult {
    scratch.tool("uname", "echo Darwin")?;
    scratch.tool("cargo", "exit 0")?;
    scratch.tool("codesign", "exit 0")?;
    scratch.tool(
        "security",
        r#"case " $* " in
*" -v "*) echo '     0 valid identities found' ;;
*) echo '  1) 0123456789ABCDEF "Expired Dev" (CSSMERR_TP_CERT_EXPIRED)' ;;
esac"#,
    )
}

#[test]
fn expired_identity_stops_before_the_build() -> TestResult {
    let scratch = TestDirectory::new("expired")?;
    fake_mac(&scratch)?;
    let root = scratch.0.join("root").display().to_string();
    let out = scratch.run(
        &repo().join("scripts/install-from-source.sh"),
        &[],
        &[
            ("HOLST_SIGN_IDENTITY", "Expired Dev"),
            ("CARGO_INSTALL_ROOT", &root),
        ],
    )?;
    let text = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("no valid code-signing identity \"Expired Dev\""),
        "{text}"
    );
    let calls = scratch.calls();
    assert!(
        calls.contains("security find-identity -v -p codesigning"),
        "{calls}"
    );
    assert!(!calls.contains("cargo install"), "built anyway:\n{calls}");
    Ok(())
}

#[test]
fn mcp_hint_quotes_the_path() -> TestResult {
    let scratch = TestDirectory::new("quote")?;
    fake_mac(&scratch)?;
    let root = scratch.0.join("with space").display().to_string();
    let out = scratch.run(
        &repo().join("scripts/install-from-source.sh"),
        &[],
        &[("CARGO_INSTALL_ROOT", &root)],
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.contains(&format!(
            "claude mcp add holst-rs -s user -- '{root}/bin/holst-mcp'\n"
        )),
        "{text}"
    );
    Ok(())
}

#[test]
fn cross_check_without_checks_explains_and_mounts_nothing() -> TestResult {
    let scratch = TestDirectory::new("cross")?;
    let scripts = scratch.0.join("root/scripts");
    fs::create_dir_all(&scripts)?;
    let script = scripts.join("cross-check.sh");
    fs::copy(repo().join("scripts/cross-check.sh"), &script)?;
    scratch.tool("git", "exit 1")?;
    scratch.tool("docker", r#"case "$1" in version) echo arm64 ;; esac"#)?;
    let logs = scratch.0.join("logs").display().to_string();
    let env = [("HOLST_XC_LOGS", logs.as_str())];
    let install = scratch.run(&script, &["install"], &env)?;
    let installed = String::from_utf8_lossy(&install.stdout);
    assert_eq!(install.status.code(), Some(0), "{installed}");
    let linux = scratch.run(&script, &["linux"], &env)?;
    let text = String::from_utf8_lossy(&linux.stdout);
    assert_eq!(linux.status.code(), Some(1), "{text}");
    assert!(
        text.contains("no checks directory found (a directory holding cli-e2e.mjs and mock.py, not shipped): set HOLST_XC_CHECKS"),
        "{text}"
    );
    let calls = scratch.calls();
    assert!(calls.contains("docker run"), "{calls}");
    assert!(
        !calls.contains("/checks:ro"),
        "an empty /checks mount:\n{calls}"
    );
    assert_eq!(
        calls.matches("docker run").count(),
        1,
        "linux ran before the check:\n{calls}"
    );
    Ok(())
}
