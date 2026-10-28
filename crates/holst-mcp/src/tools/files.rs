//! Local files: the check of a file to upload and of a place to write to.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug)]
pub(super) struct LocalFile;

impl LocalFile {
    pub(super) fn check(path: &Path, secrets: &SecretPaths) -> Result<()> {
        let real = std::fs::canonicalize(path)
            .map_err(|source| Error::Unreadable { source })?;
        match (real.is_file(), secrets.holds(&real)) {
            (false, _) => Err(Error::NotAFile),
            (true, true) => Err(Error::Secret {
                path: path.to_path_buf(),
            }),
            (true, false) => Ok(()),
        }
    }
}

// The name of the lock the core keeps beside the login file.
const LOCK_FILE: &str = "LOCK";

const SECRET_DIRS: [&str; 8] = [
    ".ssh",
    ".gnupg",
    ".config/holst",
    ".aws",
    ".netrc",
    ".docker",
    ".kube",
    ".config/gh",
];

/// Places no file may be read from or written into: the keys and the login.
#[derive(Debug, Clone, Default)]
pub(crate) struct SecretPaths(Vec<PathBuf>);

impl SecretPaths {
    /// Collects the secret directories of `home`, the login file and its lock;
    /// never the folder of the login, which `HOLST_HOME` may make the home.
    pub(crate) fn new(session_file: Option<&Path>, home: Option<&Path>) -> Self {
        let in_home = home
            .into_iter()
            .flat_map(|home| SECRET_DIRS.iter().map(move |dir| home.join(dir)));
        let login = session_file
            .into_iter()
            .filter(|file| !file.as_os_str().is_empty())
            .flat_map(|file| [file.to_path_buf(), file.with_file_name(LOCK_FILE)]);
        Self(in_home.chain(login).collect())
    }

    /// Tells whether a canonical path lies in a secret place or is one.
    fn holds(&self, real: &Path) -> bool {
        self.0
            .iter()
            .map(|place| real_path(place))
            .any(|place| real.starts_with(place))
    }
}

/// Returns `place` with the links of its existing part resolved.
fn real_path(place: &Path) -> PathBuf {
    std::fs::canonicalize(place).unwrap_or_else(|_| {
        match (place.parent(), place.file_name()) {
            (Some(parent), Some(name)) => real_path(parent).join(name),
            _ => place.to_path_buf(),
        }
    })
}

pub(super) fn out_file(
    path: &Path,
    name: &str,
    secrets: &SecretPaths,
) -> Result<PathBuf> {
    let target = if path.is_dir() {
        path.join(name)
    } else {
        path.to_path_buf()
    };
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let real =
        std::fs::canonicalize(parent).map_err(|source| Error::Unreadable { source })?;
    let planted = target
        .file_name()
        .map_or_else(|| real.clone(), |file_name| real.join(file_name));
    if secrets.holds(&planted) {
        Err(Error::SecretPlace { path: target })
    } else {
        Ok(target)
    }
}

/// Refuses a place the keys or the login are, for a file or folder to be made
/// there by the core.
pub(super) fn checked_place(path: &str, secrets: &SecretPaths) -> Result<()> {
    out_file(Path::new(path), "board", secrets).map(drop)
}

/// Refuses a download whose file, or whose directory, is a secret place.
pub(crate) fn checked_download(
    file: Option<&Path>,
    dir: Option<&Path>,
    secrets: &SecretPaths,
) -> Result<()> {
    match (file, dir) {
        (Some(file), _) => out_file(file, "original", secrets).map(drop),
        (None, Some(dir)) if secrets.holds(&real_path(dir)) => {
            Err(Error::SecretPlace {
                path: dir.to_path_buf(),
            })
        }
        (None, Some(_) | None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use std::path::{Path, PathBuf};

    use crate::{
        error::{Error, Result},
        tools::files::{LocalFile, SecretPaths, checked_download, out_file},
    };

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("holst-mcp-{tag}-{}", std::process::id()))
    }

    fn home_with(tag: &str, file: &str) -> Result<(PathBuf, PathBuf)> {
        let home = scratch(tag);
        let secret = home.join(file);
        std::fs::create_dir_all(secret.parent().unwrap_or(&home))?;
        std::fs::write(&secret, b"x")?;
        Ok((home, secret))
    }

    fn secrets_of(home: &Path) -> SecretPaths {
        SecretPaths::new(Some(&home.join(".config/holst/token")), Some(home))
    }

    #[test]
    fn upload_refuses_missing_files_and_plain_files_pass() -> Result<()> {
        let secrets = SecretPaths::default();
        assert!(matches!(
            LocalFile::check(Path::new("/no/such/file"), &secrets),
            Err(Error::Unreadable { .. })
        ));
        let tmp = scratch("upload");
        std::fs::write(&tmp, b"x")?;
        let checked = LocalFile::check(&tmp, &secrets);
        std::fs::remove_file(&tmp)?;
        assert!(checked.is_ok(), "{checked:?}");
        Ok(())
    }

    #[test]
    fn upload_refuses_keys_in_home() -> Result<()> {
        let (home, key) = home_with("gnupg-home", ".gnupg/secret.key")?;
        let checked = LocalFile::check(&key, &secrets_of(&home));
        std::fs::remove_dir_all(&home)?;
        assert!(matches!(checked, Err(Error::Secret { .. })), "{checked:?}");
        Ok(())
    }

    #[test]
    fn upload_refuses_the_login_of_a_holst_home() -> Result<()> {
        let (dir, session) = home_with("holst-home", "session")?;
        let location = holst_core::effect::settings::SessionLocation::Dir(dir.clone());
        let secrets = SecretPaths::new(location.file().as_deref(), None);
        let checked = LocalFile::check(&session, &secrets);
        std::fs::remove_dir_all(&dir)?;
        assert!(matches!(checked, Err(Error::Secret { .. })), "{checked:?}");
        Ok(())
    }

    #[test]
    fn holst_home_as_the_home_protects_only_the_login_and_its_lock() -> Result<()> {
        let (home, session) = home_with("home-is-holst-home", "session")?;
        let notes = home.join("notes.txt");
        let lock = home.join("LOCK");
        std::fs::write(&notes, b"x")?;
        std::fs::write(&lock, b"x")?;
        let location = holst_core::effect::settings::SessionLocation::Dir(home.clone());
        let secrets = SecretPaths::new(location.file().as_deref(), Some(&home));
        let upload_notes = LocalFile::check(&notes, &secrets);
        let upload_session = LocalFile::check(&session, &secrets);
        let upload_lock = LocalFile::check(&lock, &secrets);
        let download_picture = out_file(&home, "a.png", &secrets);
        let download_login = out_file(&home, "session", &secrets);
        let download_lock = out_file(&home.join("LOCK"), "a.png", &secrets);
        std::fs::remove_dir_all(&home)?;
        assert!(upload_notes.is_ok(), "{upload_notes:?}");
        assert!(
            matches!(upload_session, Err(Error::Secret { .. })),
            "{upload_session:?}"
        );
        assert!(
            matches!(upload_lock, Err(Error::Secret { .. })),
            "{upload_lock:?}"
        );
        assert_eq!(download_picture?, home.join("a.png"));
        assert!(
            matches!(download_login, Err(Error::SecretPlace { .. })),
            "{download_login:?}"
        );
        assert!(
            matches!(download_lock, Err(Error::SecretPlace { .. })),
            "{download_lock:?}"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn upload_refuses_a_link_into_a_secret_place() -> Result<()> {
        let (home, key) = home_with("link-home", ".gnupg/secret.key")?;
        let link = home.join("innocent.txt");
        std::os::unix::fs::symlink(&key, &link)?;
        let checked = LocalFile::check(&link, &secrets_of(&home));
        std::fs::remove_dir_all(&home)?;
        assert!(matches!(checked, Err(Error::Secret { .. })), "{checked:?}");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn downloads_avoid_a_login_lock_that_does_not_exist_behind_a_link() -> Result<()> {
        let root = scratch("linked-lock");
        let real = root.join("real");
        let link = root.join("link");
        std::fs::create_dir_all(&real)?;
        std::os::unix::fs::symlink(&real, &link)?;
        let secrets = SecretPaths::new(Some(&link.join("session")), None);
        let into_lock = out_file(&real, "LOCK", &secrets);
        let into_login = out_file(&real.join("session"), "a.png", &secrets);
        let into_picture = out_file(&real, "a.png", &secrets);
        std::fs::remove_dir_all(&root)?;
        assert!(
            matches!(into_lock, Err(Error::SecretPlace { .. })),
            "{into_lock:?}"
        );
        assert!(
            matches!(into_login, Err(Error::SecretPlace { .. })),
            "{into_login:?}"
        );
        assert!(into_picture.is_ok(), "{into_picture:?}");
        Ok(())
    }

    #[test]
    fn downloads_avoid_secret_places() -> Result<()> {
        let (home, key) = home_with("download-home", ".gnupg/secret.key")?;
        let secrets = secrets_of(&home);
        let into_keys = out_file(&home.join(".gnupg/new.key"), "a.png", &secrets);
        let into_dir = out_file(&home.join(".gnupg"), "a.png", &secrets);
        std::fs::remove_dir_all(&home)?;
        assert!(
            matches!(into_keys, Err(Error::SecretPlace { .. })),
            "{into_keys:?}"
        );
        assert!(
            matches!(into_dir, Err(Error::SecretPlace { .. })),
            "{into_dir:?}"
        );
        assert!(key.starts_with(scratch("download-home")));
        let tmp = std::env::temp_dir();
        assert_eq!(
            out_file(&tmp, "a.png", &SecretPaths::default())?,
            tmp.join("a.png")
        );
        Ok(())
    }

    #[test]
    fn downloads_into_a_secret_folder_are_refused() -> Result<()> {
        let (home, _key) = home_with("download-folder", ".gnupg/secret.key")?;
        let secrets = secrets_of(&home);
        let into_keys = checked_download(None, Some(&home.join(".gnupg")), &secrets);
        let below_keys =
            checked_download(None, Some(&home.join(".gnupg/new")), &secrets);
        let into_home = checked_download(None, Some(&home), &secrets);
        let nowhere = checked_download(None, None, &secrets);
        std::fs::remove_dir_all(&home)?;
        assert!(
            matches!(into_keys, Err(Error::SecretPlace { .. })),
            "{into_keys:?}"
        );
        assert!(
            matches!(below_keys, Err(Error::SecretPlace { .. })),
            "{below_keys:?}"
        );
        assert!(into_home.is_ok(), "{into_home:?}");
        assert!(nowhere.is_ok(), "{nowhere:?}");
        Ok(())
    }

    #[rstest::rstest]
    #[case::aws(".aws/credentials")]
    #[case::netrc(".netrc")]
    #[case::docker(".docker/config.json")]
    #[case::kube(".kube/config")]
    #[case::gh(".config/gh/hosts.yml")]
    fn upload_refuses_tool_logins_in_home(#[case] file: &str) -> Result<()> {
        let (home, secret) =
            home_with(&format!("logins{}", file.replace('/', "-")), file)?;
        let checked = LocalFile::check(&secret, &secrets_of(&home));
        std::fs::remove_dir_all(&home)?;
        assert!(matches!(checked, Err(Error::Secret { .. })), "{checked:?}");
        Ok(())
    }

    #[test]
    fn directories_are_not_uploaded() {
        assert!(matches!(
            LocalFile::check(&std::env::temp_dir(), &SecretPaths::default()),
            Err(Error::NotAFile)
        ));
    }
}
