//! Writing files to the local disk: whole or not at all.

use std::{
    fs,
    fs::{File, OpenOptions},
    io,
    io::Write as _,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{Error, Result, error::local_data::LocalDataError};

static PARTIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

const PARTIAL_ATTEMPTS: u32 = 16;

const PARTIAL_PREFIX: &str = ".holst-";

const PARTIAL_SUFFIX: &str = ".partial";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileAccess {
    Private,
    Shared,
}

/// Write a new file, never overwriting: a temp file is hard-linked into place;
/// without hard links (FAT) it is written directly and removed on failure.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::AlreadyExists`],
/// [`crate::error::local_data::LocalDataError::NoFileName`]
/// or [`crate::error::local_data::LocalDataError::FileWrite`].
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    write_new_with(path, bytes, |partial, to| fs::hard_link(partial, to))
}

/// [`write_new`] with the step that puts the temp file in place given.
fn write_new_with(
    path: &Path,
    bytes: &[u8],
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<()> {
    let (partial, file) = create_partial(path, FileAccess::Shared)?;
    let linked =
        finish(file, |file| file.write_all(bytes)).and_then(|()| link(&partial, path));
    // Not needed after the link or a failure; `path` stays whole anyway.
    remove_leftover(&partial);
    linked.or_else(|err| {
        match (
            err.kind() == io::ErrorKind::AlreadyExists,
            is_hard_link_unsupported(&err),
        ) {
            (true, _) => Err(already_exists(path)),
            (false, true) => write_directly(path, |out| out.write_all(bytes)),
            (false, false) => Err(write_error(path, err)),
        }
    })
}

/// Replace a file with `chunks` one after another, readable by the owner only;
/// a reader sees the old file or the new one, never a part.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::NoFileName`] or
/// [`crate::error::local_data::LocalDataError::FileWrite`]; the old file is
/// then left as it was.
pub fn replace_private(path: &Path, chunks: &[&[u8]]) -> Result<()> {
    replace_with(path, |file| {
        chunks.iter().try_for_each(|chunk| file.write_all(chunk))
    })
}

fn remove_leftover(path: &Path) {
    if let Err(err) = fs::remove_file(path) {
        tracing::debug!(%err, path = %path.display(), "temporary file not removed");
    }
}

fn replace_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<()> {
    let (partial, file) = create_partial(path, FileAccess::Private)?;
    finish(file, write)
        .and_then(|()| fs::rename(&partial, path))
        .map_err(|err| {
            remove_leftover(&partial);
            write_error(path, err)
        })
}

/// Whether `name` is a temp file of this module: `.holst-<pid>-<n>.partial`.
///
/// ```
/// use holst_core::storage::file::is_partial;
///
/// assert!(is_partial(".holst-12-3.partial"));
/// assert!(!is_partial(".holst-x-3.partial"));
/// assert!(!is_partial("token"));
/// ```
#[must_use]
pub fn is_partial(name: &str) -> bool {
    is_pid_stamped(name, PARTIAL_PREFIX, PARTIAL_SUFFIX)
}

/// Whether `name` is `prefix`, a process id, `-`, a number, `suffix`.
pub(crate) fn is_pid_stamped(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .and_then(|middle| middle.split_once('-'))
        .is_some_and(|(pid, n)| {
            [pid, n].iter().all(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
}

fn create_partial(path: &Path, access: FileAccess) -> Result<(PathBuf, File)> {
    path.file_name().ok_or_else(|| {
        Error::LocalData(LocalDataError::NoFileName {
            path: path.to_path_buf(),
        })
    })?;
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    first_free(dir, |partial| open_new(partial, access))
        .map_err(|err| write_error(path, err))
}

/// A hidden folder to build in, removed on drop, placed by [`place_dir`].
#[derive(Debug)]
pub struct PartialDir(PathBuf);

impl PartialDir {
    /// Make the folder in `parent`, made too when missing.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::FileWrite`].
    pub fn create(parent: &Path) -> Result<Self> {
        create_dir_all(parent)?;
        first_free(parent, |partial| fs::create_dir(partial))
            .map(|(partial, ())| Self(partial))
            .map_err(|err| write_error(parent, err))
    }

    /// Returns where the folder is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

// GUARD: removes the partial folder when the command ends early.
impl Drop for PartialDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap_or_default();
    }
}

/// Moves `partial` to the first free `stem`, `stem-2`, …: each name is claimed
/// with `create_dir`, which never takes a name that exists, so a folder made
/// meanwhile is not replaced.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::AlreadyExists`] when 999 names
/// are taken, or [`crate::error::local_data::LocalDataError::FileWrite`].
pub fn place_dir(parent: &Path, stem: &str, partial: &PartialDir) -> Result<PathBuf> {
    let placed =
        numbered(parent, stem, "").find_map(|path| match fs::create_dir(&path) {
            Ok(()) => Some(
                move_onto(partial.path(), &path, |from, to| fs::rename(from, to))
                    .map(|()| path.clone())
                    .map_err(|source| {
                        fs::remove_dir(&path).unwrap_or_default();
                        write_error(&path, source)
                    }),
            ),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => None,
            Err(source) => Some(Err(write_error(&path, source))),
        });
    placed.unwrap_or_else(|| Err(already_exists(&parent.join(stem))))
}

/// Renames `from` onto the empty folder `to`; where the file system refuses
/// (Windows without POSIX rename: FAT32, exFAT) frees the name and renames once
/// more.
fn move_onto(
    from: &Path,
    to: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    rename(from, to).or_else(|_| {
        fs::remove_dir(to)?;
        rename(from, to)
    })
}

/// Write a new file under the first free `stem.extension`, `stem-2.extension`,
/// … in `dir`, never overwriting; `dir` is not made.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::AlreadyExists`] when 999 names
/// are taken, or [`crate::error::local_data::LocalDataError::FileWrite`].
pub fn write_numbered(
    dir: &Path,
    stem: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    let written =
        numbered(dir, stem, extension).find_map(|path| match write_new(&path, bytes) {
            Ok(()) => Some(Ok(path)),
            Err(Error::LocalData(LocalDataError::AlreadyExists { .. })) => None,
            Err(err) => Some(Err(err)),
        });
    written.unwrap_or_else(|| Err(already_exists(&dir.join(stem))))
}

const MAX_NUMBERED: u32 = 999;

fn numbered<'a>(
    dir: &'a Path,
    stem: &'a str,
    extension: &'a str,
) -> impl Iterator<Item = PathBuf> + 'a {
    let dot = match extension {
        "" => String::new(),
        extension => format!(".{extension}"),
    };
    (1..=MAX_NUMBERED).map(move |number| match number {
        1 => dir.join(format!("{stem}{dot}")),
        number => dir.join(format!("{stem}-{number}{dot}")),
    })
}

/// Make `path` and its missing parents.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::FileWrite`].
pub fn create_dir_all(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|err| write_error(path, err))
}

/// Make the folder `path` in an existing parent.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::FileWrite`].
pub fn create_dir(path: &Path) -> Result<()> {
    fs::create_dir(path).map_err(|err| write_error(path, err))
}

/// Makes the first partial name in `dir` that `make` finds free.
fn first_free<Made>(
    dir: &Path,
    make: impl Fn(&Path) -> io::Result<Made>,
) -> io::Result<(PathBuf, Made)> {
    let outcome = (0..PARTIAL_ATTEMPTS).try_fold(
        io::Error::from(io::ErrorKind::AlreadyExists),
        |_, _| {
            let partial = dir.join(partial_name());
            match make(&partial) {
                Ok(opened) => ControlFlow::Break(Ok((partial, opened))),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    ControlFlow::Continue(err)
                }
                Err(err) => ControlFlow::Break(Err(err)),
            }
        },
    );
    match outcome {
        ControlFlow::Break(created) => created,
        ControlFlow::Continue(err) => Err(err),
    }
}

fn partial_name() -> String {
    let n = PARTIAL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{PARTIAL_PREFIX}{}-{n}{PARTIAL_SUFFIX}", std::process::id())
}

fn open_new(path: &Path, access: FileAccess) -> io::Result<File> {
    restricted(OpenOptions::new().write(true).create_new(true), access).open(path)
}

#[cfg(unix)]
fn restricted(builder: &mut OpenOptions, access: FileAccess) -> &mut OpenOptions {
    match access {
        FileAccess::Private => std::os::unix::fs::OpenOptionsExt::mode(builder, 0o600),
        FileAccess::Shared => builder,
    }
}

#[cfg(not(unix))]
fn restricted(builder: &mut OpenOptions, _: FileAccess) -> &mut OpenOptions {
    builder
}

fn finish(
    mut file: File,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    write(&mut file)?;
    file.sync_all()
}

fn is_hard_link_unsupported(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
    )
}

fn write_directly(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<()> {
    let file = File::create_new(path).map_err(|err| {
        if err.kind() == io::ErrorKind::AlreadyExists {
            already_exists(path)
        } else {
            write_error(path, err)
        }
    })?;
    finish(file, write).map_err(|err| {
        remove_leftover(path);
        write_error(path, err)
    })
}

fn already_exists(path: &Path) -> Error {
    Error::LocalData(LocalDataError::AlreadyExists {
        path: path.to_path_buf(),
    })
}

/// The bytes of the local file `path`.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::FileRead`].
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    fs::read(path).map_err(|error| {
        Error::LocalData(LocalDataError::FileRead {
            path: path.to_path_buf(),
            error,
        })
    })
}

pub(crate) fn write_error(path: &Path, error: io::Error) -> Error {
    Error::LocalData(LocalDataError::FileWrite {
        path: path.to_path_buf(),
        error,
    })
}

pub(crate) fn name_of(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

pub(crate) fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect()
}

pub(crate) fn subdirs(dir: &Path) -> Vec<PathBuf> {
    entries(dir)
        .into_iter()
        .filter(|path| fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()))
        .collect()
}

pub(crate) fn tree_size(path: &Path) -> u64 {
    fs::symlink_metadata(path).map_or(0, |meta| {
        if meta.is_dir() {
            entries(path).iter().map(|entry| tree_size(entry)).sum()
        } else {
            meta.len()
        }
    })
}

pub(crate) fn byte_len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

#[cfg(unix)]
pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    std::os::unix::fs::DirBuilderExt::mode(fs::DirBuilder::new().recursive(true), 0o700)
        .create(path)
}

#[cfg(not(unix))]
pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).create(path)
}

#[cfg(unix)]
pub(crate) fn tighten(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let meta = fs::symlink_metadata(dir)?;
    let loose = meta.is_dir() && meta.permissions().mode() & 0o777 != 0o700;
    loose.then_some(dir).map_or(Ok(()), |dir| {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
    })
}

#[cfg(not(unix))]
pub(crate) fn tighten(dir: &Path) -> io::Result<()> {
    fs::symlink_metadata(dir).map(drop)
}

#[cfg(unix)]
pub(crate) fn private_file(path: &Path) -> io::Result<File> {
    std::os::unix::fs::OpenOptionsExt::mode(
        OpenOptions::new().create(true).truncate(false).write(true),
        0o600,
    )
    .open(path)
}

#[cfg(not(unix))]
pub(crate) fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
}

pub(crate) fn remove_file(path: &Path) -> Result<()> {
    fs::remove_file(path).or_else(|err| gone_or(path, err))
}

pub(crate) fn remove_dir(path: &Path) -> Result<()> {
    fs::remove_dir_all(path).or_else(|err| gone_or(path, err))
}

pub(crate) fn remove_any(path: &Path) -> Result<()> {
    let dir = fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    if dir {
        remove_dir(path)
    } else {
        remove_file(path)
    }
}

fn gone_or(path: &Path, err: io::Error) -> Result<()> {
    if err.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(Error::LocalData(LocalDataError::FileRemove {
            path: path.to_path_buf(),
            error: err,
        }))
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
        io::Write as _,
        path::{Path, PathBuf},
        sync::atomic::Ordering,
    };

    use itertools::Itertools as _;
    use rstest::rstest;

    use crate::{
        Error,
        Result,
        error::local_data::LocalDataError,
        storage::file::{
            PARTIAL_COUNTER,
            PartialDir,
            is_partial,
            move_onto,
            partial_name,
            place_dir,
            replace_private,
            replace_with,
            write_directly,
            write_new,
            write_new_with,
            write_numbered,
        },
    };

    #[test]
    fn numbered_files_are_never_overwritten() -> Result<()> {
        let dir = scratch("numbered")?;
        let missing = write_numbered(&dir.join("x"), "a", "png", b"1");
        assert!(
            missing.is_err(),
            "a missing directory is not made for a file"
        );
        let names: Vec<PathBuf> = (0..3)
            .map(|_| write_numbered(&dir, "a", "png", b"1"))
            .collect::<Result<_>>()?;
        assert_eq!(
            names,
            [dir.join("a.png"), dir.join("a-2.png"), dir.join("a-3.png")]
        );
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn folders_are_placed_under_free_names() -> Result<()> {
        let dir = scratch("placed")?;
        let taken = dir.join("a");
        fs::create_dir(&taken)?;
        fs::write(taken.join("mine.txt"), b"1")?;
        fs::create_dir(dir.join("a-2"))?;
        let partial = PartialDir::create(&dir)?;
        let placed = place_dir(&dir, "a", &partial)?;
        assert_eq!(placed, dir.join("a-3"));
        assert!(
            taken.join("mine.txt").is_file(),
            "an old folder was replaced"
        );
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// FAT32 and exFAT: the target of a rename must not exist as a folder.
    fn refuse_existing_folder(from: &Path, to: &Path) -> io::Result<()> {
        if to.exists() {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            fs::rename(from, to)
        }
    }

    #[test]
    fn a_rename_onto_a_folder_that_a_file_system_refuses_is_retried_on_a_free_name()
    -> Result<()> {
        let dir = scratch("move_onto")?;
        let from = dir.join("from");
        let to = dir.join("to");
        fs::create_dir(&from)?;
        fs::create_dir(&to)?;
        move_onto(&from, &to, refuse_existing_folder)?;
        assert!(to.is_dir() && !from.exists());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_folder_dropped_unplaced_leaves_nothing() -> Result<()> {
        let dir = scratch("dropped")?;
        let partial = PartialDir::create(&dir)?;
        fs::write(partial.path().join("half.txt"), b"1")?;
        drop(partial);
        assert_eq!(
            fs::read_dir(&dir)?.count(),
            0,
            "a half-filled folder stayed"
        );
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    fn not_found_is_ok(err: io::Error) -> io::Result<()> {
        if err.kind() == io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(err)
        }
    }

    fn scratch(test: &str) -> io::Result<PathBuf> {
        let dir = std::env::temp_dir()
            .join(format!("holst-core-fs-{test}-{}", std::process::id()));
        fs::remove_dir_all(&dir).or_else(not_found_is_ok)?;
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn leftovers(dir: &Path) -> io::Result<Vec<String>> {
        let names: io::Result<Vec<String>> = fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|name| name.as_ref().map_or(true, |name| is_partial(name)))
            .collect();
        names.map(|mut names| {
            names.sort_unstable();
            names
        })
    }

    #[rstest]
    #[case::short_name("new", "snapshot.bin".into())]
    #[case::near_fs_limit("long", format!("{}.bin", "б".repeat(118)))]
    fn writes_new_file(#[case] test: &str, #[case] name: String) -> Result<()> {
        assert!(test != "long" || name.len() == 240, "{}", name.len());
        let dir = scratch(test)?;
        let path = dir.join(&name);
        write_new(&path, b"payload")?;
        assert_eq!(fs::read(&path)?, b"payload");
        assert!(leftovers(&dir)?.is_empty());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn never_overwrites() -> Result<()> {
        let dir = scratch("exists")?;
        let path = dir.join("snapshot.bin");
        fs::write(&path, b"first")?;
        let err = write_new(&path, b"second").err();
        assert_eq!(
            err.map(|err| err.to_string()),
            Some(format!(
                "{} already exists; refusing to overwrite it",
                path.display()
            ))
        );
        assert_eq!(fs::read(&path)?, b"first");
        assert!(leftovers(&dir)?.is_empty());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn failure_leaves_no_partial() -> Result<()> {
        let dir = scratch("missing")?;
        let path = dir.join("no-such-dir").join("snapshot.bin");
        let err = write_new(&path, b"payload").err();
        assert_eq!(
            err.map(|err| err.to_string()),
            Some(format!("could not write {}", path.display()))
        );
        assert!(!path.exists());
        assert!(leftovers(&dir)?.is_empty());

        // A directory at the target: the link fails after the temporary file.
        let taken = dir.join("taken");
        fs::create_dir(&taken)?;
        assert!(write_new(&taken, b"payload").is_err());
        assert!(taken.is_dir());
        assert!(leftovers(&dir)?.is_empty());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn path_without_file_name_is_invalid() {
        assert!(matches!(
            write_new(Path::new("/"), b"x"),
            Err(Error::LocalData(LocalDataError::NoFileName { .. }))
        ));
        assert!(matches!(
            replace_private(Path::new("/"), &[b"x"]),
            Err(Error::LocalData(LocalDataError::NoFileName { .. }))
        ));
    }

    #[test]
    fn partial_names_are_hidden_short_and_unique() {
        let first = partial_name();
        let second = partial_name();
        assert_ne!(first, second);
        assert!(first.starts_with(".holst-"), "{first}");
        assert!(is_partial(&first), "{first}");
        assert!(first.len() <= 50, "{first}");
    }

    #[test]
    fn replaces_a_file_privately() -> Result<()> {
        let dir = scratch("replace")?;
        let path = dir.join("session");
        fs::write(&path, b"old")?;
        replace_private(&path, &[b"new", b"\n"])?;
        assert_eq!(fs::read(&path)?, b"new\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        }
        assert!(leftovers(&dir)?.is_empty());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_intact() -> Result<()> {
        let dir = scratch("crash")?;
        let path = dir.join("session");
        fs::write(&path, b"old")?;
        let err = replace_with(&path, |file| {
            file.write_all(b"half of the new")?;
            Err(io::Error::from(io::ErrorKind::StorageFull))
        })
        .err();
        assert_eq!(
            err.map(|err| err.to_string()),
            Some(format!("could not write {}", path.display()))
        );
        assert_eq!(fs::read(&path)?, b"old");
        assert!(leftovers(&dir)?.is_empty());

        // A directory in the way: the rename fails after the temporary file.
        let taken = dir.join("taken");
        fs::create_dir_all(taken.join("inside"))?;
        assert!(replace_private(&taken, &[b"x"]).is_err());
        assert!(taken.join("inside").is_dir());
        assert!(leftovers(&dir)?.is_empty());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn leftover_partials_are_ignored() -> Result<()> {
        let dir = scratch("leftover")?;
        let next = PARTIAL_COUNTER.load(Ordering::Relaxed);
        let crashed: Vec<String> = (next..next + 3)
            .map(|n| format!(".holst-{}-{n}.partial", std::process::id()))
            .chain([".holst-1-0.partial".to_owned()])
            .sorted_unstable()
            .collect();
        crashed
            .iter()
            .try_for_each(|name| fs::write(dir.join(name), b"torn"))?;
        let (session, export) = (dir.join("session"), dir.join("export.bin"));
        replace_private(&session, &[b"login"])?;
        write_new(&export, b"board")?;
        assert_eq!(fs::read(&session)?, b"login");
        assert_eq!(fs::read(&export)?, b"board");
        assert_eq!(leftovers(&dir)?, crashed);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    fn no_hard_links(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    #[test]
    fn without_hard_links_a_new_file_is_written_directly() -> Result<()> {
        let dir = scratch("direct-new")?;
        let path = dir.join("a.bin");
        write_new_with(&path, b"payload", no_hard_links)?;
        assert_eq!(fs::read(&path)?, b"payload");
        assert!(leftovers(&dir)?.is_empty(), "a temp file stayed");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn without_hard_links_an_existing_file_is_refused() -> Result<()> {
        let dir = scratch("direct-taken")?;
        let path = dir.join("a.bin");
        fs::write(&path, b"mine")?;
        let refused = write_new_with(&path, b"payload", no_hard_links);
        assert!(
            matches!(
                refused,
                Err(Error::LocalData(LocalDataError::AlreadyExists { .. }))
            ),
            "{refused:?}"
        );
        assert_eq!(fs::read(&path)?, b"mine");
        assert!(leftovers(&dir)?.is_empty(), "a temp file stayed");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_link_failure_that_is_not_missing_support_is_a_write_error() -> Result<()> {
        let dir = scratch("link-fails")?;
        let path = dir.join("a.bin");
        let failed = write_new_with(&path, b"payload", |_, _| {
            Err(io::ErrorKind::OutOfMemory.into())
        });
        assert!(
            matches!(
                failed,
                Err(Error::LocalData(LocalDataError::FileWrite { .. }))
            ),
            "{failed:?}"
        );
        assert!(!path.exists(), "a file stayed");
        assert!(leftovers(&dir)?.is_empty(), "a temp file stayed");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_direct_write_makes_the_file_and_never_replaces_one() -> Result<()> {
        let dir = scratch("direct")?;
        let path = dir.join("a.bin");
        write_directly(&path, |file| file.write_all(b"first"))?;
        assert_eq!(fs::read(&path)?, b"first");
        let refused = write_directly(&path, |file| file.write_all(b"second"));
        assert!(
            matches!(
                refused,
                Err(Error::LocalData(LocalDataError::AlreadyExists { .. }))
            ),
            "{refused:?}"
        );
        assert_eq!(fs::read(&path)?, b"first");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_failed_direct_write_leaves_no_file() -> Result<()> {
        let dir = scratch("direct-fails")?;
        let path = dir.join("a.bin");
        let failed = write_directly(&path, |file| {
            file.write_all(b"half")?;
            Err(io::ErrorKind::StorageFull.into())
        });
        assert!(
            matches!(
                failed,
                Err(Error::LocalData(LocalDataError::FileWrite { .. }))
            ),
            "{failed:?}"
        );
        assert!(!path.exists(), "a half-written file stayed");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
