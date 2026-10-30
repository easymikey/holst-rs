use std::process::ExitCode;

use holst_core::{
    Error as CoreError,
    error::{
        board_change::BoardChangeError,
        board_socket::{BoardSocketError, SocketErrorCode},
        http::HttpError,
        input::InputError,
        local_data::{KeychainError, LocalDataError},
        login::LoginError,
        lookup::{LookupError, LookupMiss},
        picture::PictureError,
        stored_file::StoredFileError,
    },
};

use crate::{error::Error, terminal::signals::Signal};

/// Success; also a closed stdout, where the reader got what it wanted.
pub(crate) const EX_OK: u8 = 0;
/// Bad arguments or flags, or a name that fits several things.
pub(crate) const EX_USAGE: u8 = 64;
/// Bad input data.
pub(crate) const EX_DATAERR: u8 = 65;
/// A named thing does not exist.
pub(crate) const EX_NOINPUT: u8 = 66;
/// The server refused or is down.
pub(crate) const EX_UNAVAILABLE: u8 = 69;
/// An output file exists or cannot be made.
pub(crate) const EX_CANTCREAT: u8 = 73;
/// Reading or writing failed.
pub(crate) const EX_IOERR: u8 = 74;
/// A retry may help: timeouts, 5xx, rate limits.
pub(crate) const EX_TEMPFAIL: u8 = 75;
/// Not logged in, no access, or a write that needs `--yes`.
pub(crate) const EX_NOPERM: u8 = 77;
/// Bad environment or settings.
pub(crate) const EX_CONFIG: u8 = 78;
/// Stopped by Ctrl-C.
pub(crate) const EXIT_INTERRUPTED: u8 = Signal::Interrupt.exit_code();

/// The exit-code table in `holst --help`: code and meaning.
const EXIT_CODES: [(u8, &str); 13] = [
    (EX_OK, "success"),
    (
        EX_USAGE,
        "bad arguments or flags, or a name that fits several things",
    ),
    (EX_DATAERR, "bad input data"),
    (EX_NOINPUT, "a named thing does not exist"),
    (EX_UNAVAILABLE, "the server refused or is down"),
    (EX_CANTCREAT, "an output file exists or cannot be made"),
    (EX_IOERR, "reading or writing failed"),
    (EX_TEMPFAIL, "a retry may help: timeouts, 5xx, rate limits"),
    (
        EX_NOPERM,
        "not logged in, no access, or a write that needs `--yes`",
    ),
    (EX_CONFIG, "bad environment or settings"),
    (
        Signal::HangUp.exit_code(),
        "stopped by the terminal closing (SIGHUP)",
    ),
    (EXIT_INTERRUPTED, "stopped by Ctrl-C"),
    (Signal::Terminate.exit_code(), "stopped by SIGTERM"),
];

/// Returns the `EXIT CODES` block of the top-level help.
pub(crate) fn help_block() -> String {
    EXIT_CODES
        .iter()
        .fold("EXIT CODES:".to_owned(), |block, (code, meaning)| {
            format!("{block}\n  {code:>3}  {meaning}")
        })
}

/// Prints a command-line parse error or the help text and returns the exit
/// code.
pub(crate) fn usage(error: &clap::Error) -> ExitCode {
    // A closed stderr leaves nobody to tell; the code still says it.
    drop(error.print());
    // clap's own code is 0 for `--help` and `--version`, and 2 for a usage error.
    ExitCode::from(match error.exit_code() {
        0 => EX_OK,
        _ => EX_USAGE,
    })
}

pub(crate) fn report(error: &Error) -> ExitCode {
    let code = ExitCode::from(code(error));
    match error {
        // The form already showed a `Shown` error under its `└`.
        Error::StdoutClosed | Error::Shown(_) => code,
        Error::Holst(_)
        | Error::ReadInput(_)
        | Error::NoInput { .. }
        | Error::WriteStdout(_)
        | Error::Terminal(_)
        | Error::EchoNotHidden(_)
        | Error::Render(_)
        | Error::BinaryToTerminal
        | Error::SvgToTerminal
        | Error::NoDownloads
        | Error::McpAlias { .. }
        | Error::Interrupted
        | Error::Terminated
        | Error::HungUp
        | Error::SessionStoreLost
        | Error::Cancelled
        | Error::InvalidAnswer { .. }
        | Error::NoAccount { .. }
        | Error::SsoOnly { .. }
        | Error::WrongSecret { .. }
        | Error::NotForgotten { .. }
        | Error::NotCleaned { .. }
        | Error::SessionNotAccepted(_)
        | Error::Unconfirmed(_)
        | Error::QueryNoEquals { .. }
        | Error::BodyFile { .. }
        | Error::ItemsNotJson(_) => print(error, code),
    }
}

/// Prints the error on stderr and passes its exit code on.
fn print(error: &Error, code: ExitCode) -> ExitCode {
    crate::output::stderr_line(crate::output::view::report(
        error,
        crate::output::stderr_coloring(),
    ));
    code
}

/// Returns the process exit code of an error, by its class (BSD `sysexits`).
pub(crate) fn code(error: &Error) -> u8 {
    match error {
        Error::Holst(inner) => core_code(inner),
        Error::StdoutClosed => EX_OK,
        Error::Shown(inner) => code(inner),
        Error::Interrupted => EXIT_INTERRUPTED,
        Error::Terminated => Signal::Terminate.exit_code(),
        Error::HungUp => Signal::HangUp.exit_code(),
        Error::BinaryToTerminal
        | Error::SvgToTerminal
        | Error::McpAlias { .. }
        | Error::QueryNoEquals { .. } => EX_USAGE,
        Error::Render(_) | Error::InvalidAnswer { .. } | Error::ItemsNotJson(_) => {
            EX_DATAERR
        }
        Error::NoInput { .. } | Error::NoAccount { .. } | Error::BodyFile { .. } => {
            EX_NOINPUT
        }
        Error::SsoOnly { .. } => EX_UNAVAILABLE,
        Error::ReadInput(_)
        | Error::WriteStdout(_)
        | Error::Terminal(_)
        | Error::EchoNotHidden(_)
        | Error::SessionStoreLost
        | Error::NotForgotten { .. }
        | Error::NotCleaned { .. } => EX_IOERR,
        Error::Cancelled => EX_TEMPFAIL,
        Error::WrongSecret { .. }
        | Error::SessionNotAccepted(_)
        | Error::Unconfirmed(_) => EX_NOPERM,
        Error::NoDownloads => EX_CONFIG,
    }
}

fn core_code(error: &CoreError) -> u8 {
    match error {
        CoreError::Login(inner) => login_code(inner),
        CoreError::Http(inner) => http_code(inner),
        CoreError::Input(inner) => input_code(inner),
        CoreError::LocalData(inner) => store_code(inner),
        CoreError::Lookup(inner) => lookup_code(inner),
        CoreError::BoardSocket(inner) => hud_code(inner),
        CoreError::BoardChange(inner) => board_write_code(inner),
        CoreError::Picture(inner) => picture_code(inner),
        CoreError::StoredFile(inner) => asset_code(inner),
    }
}

fn login_code(error: &LoginError) -> u8 {
    match error {
        LoginError::NoSavedLogin
        | LoginError::SessionRevoked
        | LoginError::OtherServer { .. }
        | LoginError::BareToken { .. } => EX_NOPERM,
        LoginError::NotEmail { .. } | LoginError::NotSavedLogin => EX_DATAERR,
        LoginError::Incomplete { .. } => EX_UNAVAILABLE,
    }
}

fn http_code(error: &HttpError) -> u8 {
    match error {
        HttpError::Status { status, .. } => status_code(*status),
        HttpError::NoResponse { .. }
        | HttpError::RateLimited { .. }
        | HttpError::Transport(_) => EX_TEMPFAIL,
        HttpError::HtmlInsteadOfJson { .. }
        | HttpError::ResponseTooLarge { .. }
        | HttpError::ForeignRedirect { .. }
        | HttpError::MissingField { .. }
        | HttpError::JsonInsteadOfBackup { .. } => EX_UNAVAILABLE,
        HttpError::Json(_) => EX_DATAERR,
    }
}

fn status_code(status: u16) -> u8 {
    match status {
        401 | 403 => EX_NOPERM,
        404 => EX_NOINPUT,
        400 | 422 => EX_DATAERR,
        408 | 429 | 500..=599 => EX_TEMPFAIL,
        _ => EX_UNAVAILABLE,
    }
}

fn input_code(error: &InputError) -> u8 {
    match error {
        InputError::Several { errors: problems } => {
            problems.first().map_or(EX_DATAERR, core_code)
        }
        InputError::BaseNotOrigin { .. }
        | InputError::BaseNotUrl { .. }
        | InputError::BaseNotHttps { .. }
        | InputError::CacheMaxNotSize { .. }
        | InputError::CacheTrustNotDuration { .. }
        | InputError::CacheDirRelative { .. } => EX_CONFIG,
        InputError::NoWorkspaces => EX_NOINPUT,
        InputError::GuestSharesNoTemplate { .. } => EX_NOPERM,
        InputError::SessionNotHeaderSafe
        | InputError::EmptySharingPatch
        | InputError::TemplateNotObject => EX_DATAERR,
        InputError::NotUrl { .. }
        | InputError::ForeignOrigin { .. }
        | InputError::NotApiPath { .. }
        | InputError::FullUrl
        | InputError::RelativePath
        | InputError::NotBoardId { .. }
        | InputError::NotWorkspaceId { .. }
        | InputError::NotFolderId { .. }
        | InputError::NotObjectId { .. }
        | InputError::NotPathSegment { .. }
        | InputError::NotFinite { .. }
        | InputError::NotPositive { .. }
        | InputError::NotNodeKind { .. }
        | InputError::NotObjectKind { .. }
        | InputError::NotIcon { .. }
        | InputError::NotHttpMethod { .. }
        | InputError::OfflineByName { .. }
        | InputError::RootOrFolder { .. }
        | InputError::BlankName
        | InputError::BlankMediaName
        | InputError::LeaveOwnBoard { .. }
        | InputError::MoveIntoItself { .. }
        | InputError::WorkspaceNotChosen { .. }
        | InputError::NotArgument { .. }
        | InputError::NoArgument { .. }
        | InputError::NotArgumentValue { .. }
        | InputError::ReplyNotFound { .. }
        | InputError::NotCommandArguments(_)
        | InputError::ImportBoardLeft { .. } => EX_USAGE,
    }
}

fn store_code(error: &LocalDataError) -> u8 {
    match error {
        LocalDataError::Keychain(inner) => keychain_code(inner),
        LocalDataError::NoFileName { .. } | LocalDataError::PlainGz { .. } => EX_USAGE,
        LocalDataError::Unpack(_)
        | LocalDataError::UnpackTooLarge { .. }
        | LocalDataError::NotBoardIndex
        | LocalDataError::IndexVersion { .. }
        | LocalDataError::IndexCorrupt { .. }
        | LocalDataError::SavedLoginCorrupt => EX_DATAERR,
        LocalDataError::NotCached { .. } => EX_NOINPUT,
        LocalDataError::AlreadyExists { .. } | LocalDataError::FileWrite { .. } => {
            EX_CANTCREAT
        }
        LocalDataError::FileRead { .. }
        | LocalDataError::FileRemove { .. }
        | LocalDataError::Pack(_)
        | LocalDataError::Io(_)
        | LocalDataError::IndexEncode(_)
        | LocalDataError::StaleLogin { .. } => EX_IOERR,
        LocalDataError::CacheBusy { .. }
        | LocalDataError::LoginBusy { .. }
        | LocalDataError::LoginReplaced => EX_TEMPFAIL,
        LocalDataError::Symlink { .. }
        | LocalDataError::OutsideLocalData { .. }
        | LocalDataError::AccountUnknown
        | LocalDataError::KeychainLocked => EX_NOPERM,
        LocalDataError::CacheOff | LocalDataError::NoHome => EX_CONFIG,
        #[cfg(not(target_os = "macos"))]
        LocalDataError::NoSecretService(_)
        | LocalDataError::SecretService(_)
        | LocalDataError::KeychainUnavailable(_) => EX_UNAVAILABLE,
    }
}

fn keychain_code(error: &KeychainError) -> u8 {
    match error {
        KeychainError::Locked | KeychainError::Denied | KeychainError::Cancelled => {
            EX_NOPERM
        }
        KeychainError::TimedOut(_) => EX_TEMPFAIL,
        KeychainError::TooLong { .. } | KeychainError::BadName => EX_DATAERR,
        KeychainError::Spawn { .. } => EX_UNAVAILABLE,
        KeychainError::Exited { .. } | KeychainError::Killed => EX_IOERR,
    }
}

fn lookup_code(error: &LookupError) -> u8 {
    match error.miss {
        LookupMiss::NotFound { .. } | LookupMiss::Elsewhere { .. } => EX_NOINPUT,
        LookupMiss::Ambiguous { .. } | LookupMiss::Uncertain { .. } => EX_USAGE,
    }
}

fn hud_code(error: &BoardSocketError) -> u8 {
    match error {
        BoardSocketError::NotSubscribed { code: refusal } => socket_exit_code(refusal),
        BoardSocketError::Closed
        | BoardSocketError::Timeout { .. }
        | BoardSocketError::TotalTimeout { .. }
        | BoardSocketError::Connection(_)
        | BoardSocketError::CopyAhead
        | BoardSocketError::Incomplete => EX_TEMPFAIL,
        BoardSocketError::MalformedFrame { .. }
        | BoardSocketError::ForeignLink { .. }
        | BoardSocketError::Proxy { .. }
        | BoardSocketError::ProxyDenied { .. }
        | BoardSocketError::UnsupportedFormat { .. } => EX_UNAVAILABLE,
        BoardSocketError::Decode(_) | BoardSocketError::UpdateNotMerged(_) => {
            EX_DATAERR
        }
        BoardSocketError::ProxyScheme { .. } => EX_CONFIG,
    }
}

/// Returns the code for the refusal the board server gave for the board.
fn socket_exit_code(refusal: &SocketErrorCode) -> u8 {
    match refusal {
        SocketErrorCode::Forbidden => EX_NOPERM,
        SocketErrorCode::NotFound => EX_NOINPUT,
        SocketErrorCode::InternalError => EX_TEMPFAIL,
        SocketErrorCode::Other(_) => EX_UNAVAILABLE,
    }
}

fn board_write_code(error: &BoardChangeError) -> u8 {
    match error {
        BoardChangeError::ReadOnly
        | BoardChangeError::CopyWithheld { .. }
        | BoardChangeError::LockWithheld { .. }
        | BoardChangeError::HideWithheld { .. }
        | BoardChangeError::DeleteWithheld { .. }
        | BoardChangeError::NotEmpty { .. }
        | BoardChangeError::Locked { .. }
        | BoardChangeError::NotYours { .. } => EX_NOPERM,
        BoardChangeError::NoObject { .. }
        | BoardChangeError::NotChangeable { .. }
        | BoardChangeError::NotSpinnable { .. }
        | BoardChangeError::NotMovable { .. }
        | BoardChangeError::PlacedByContainer { .. }
        | BoardChangeError::NothingToChange
        | BoardChangeError::NoTag { .. }
        | BoardChangeError::TagTaken { .. }
        | BoardChangeError::NoCheckItem { .. }
        | BoardChangeError::NoPage { .. }
        | BoardChangeError::NotPoint { .. }
        | BoardChangeError::OffBoard { .. }
        | BoardChangeError::NotSize { .. }
        | BoardChangeError::NotColor { .. }
        | BoardChangeError::NotArrowLook { .. }
        | BoardChangeError::BadValue { .. }
        | BoardChangeError::StickerSize { .. }
        | BoardChangeError::NotFrame { .. }
        | BoardChangeError::ContentHidden { .. }
        | BoardChangeError::TooFewToGroup { .. }
        | BoardChangeError::NotGroupable { .. }
        | BoardChangeError::NotGroup { .. }
        | BoardChangeError::NotLockable { .. }
        | BoardChangeError::NotLocked { .. }
        | BoardChangeError::NothingToUnlock
        | BoardChangeError::NotArrangeable { .. }
        | BoardChangeError::TooFewToArrange { .. }
        | BoardChangeError::TableRefused { .. }
        | BoardChangeError::OutsideFrame { .. } => EX_USAGE,
        BoardChangeError::AcceptTimeout { .. }
        | BoardChangeError::ClosedBeforeAccept
        | BoardChangeError::NotAccepted { .. } => EX_TEMPFAIL,
    }
}

fn picture_code(error: &PictureError) -> u8 {
    match error {
        PictureError::NotArea { .. }
        | PictureError::SeveralAreas
        | PictureError::OutOfRange { .. }
        | PictureError::TooLarge { .. } => EX_USAGE,
        PictureError::EmptyBoard | PictureError::EmptyArea { .. } => EX_DATAERR,
        #[cfg(feature = "png")]
        PictureError::SvgParse(_) => EX_DATAERR,
        PictureError::PngEncode => EX_IOERR,
    }
}

fn asset_code(error: &StoredFileError) -> u8 {
    match error {
        StoredFileError::NoFile { .. } | StoredFileError::NoObject { .. } => EX_USAGE,
        StoredFileError::ForeignStorage { .. } => EX_DATAERR,
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use holst_core::{
        error::{
            board_change::BoardChangeError,
            board_socket::{BoardSocketError, SocketErrorCode},
            confirmation::{ChangeRisk, Unconfirmed},
            http::HttpError,
            input::InputError,
            local_data::{KeychainError, LocalDataError},
            login::LoginError,
            lookup::{LookupError, LookupKind, LookupMiss},
        },
        fake::candidates,
    };
    use rstest::rstest;

    use crate::{error::Error, exit::code};

    fn core(error: impl Into<holst_core::Error>) -> Error {
        Error::Holst(error.into())
    }

    fn shown_core(error: impl Into<holst_core::Error>) -> Error {
        core(error).shown()
    }

    fn refused(code: u16) -> holst_core::Error {
        HttpError::Status {
            method: "GET",
            path: "/gapi/current_user".into(),
            status: code,
            detail: String::new(),
        }
        .into()
    }

    fn status(code: u16) -> Error {
        Error::Holst(refused(code))
    }

    fn missing(outcome: LookupMiss) -> Error {
        core(LookupError {
            kind: LookupKind::Board,
            reference: "x".into(),
            miss: outcome,
        })
    }

    #[rstest]
    #[case::stdout_closed(Error::StdoutClosed, 0)]
    #[case::interrupted(Error::Interrupted, 130)]
    #[case::interrupt_shown(Error::Interrupted.shown(), 130)]
    #[case::terminated(Error::Terminated, 143)]
    #[case::hung_up(Error::HungUp, 129)]
    #[case::no_token_shown(shown_core(LoginError::NoSavedLogin), 77)]
    #[case::no_input_shown(Error::NoInput { what: "a code" }.shown(), 66)]
    #[case::terminal_binary(Error::BinaryToTerminal, 64)]
    #[case::unknown_kind(core(InputError::NotNodeKind { raw: "x".into() }), 64)]
    #[case::ambiguous(missing(LookupMiss::ambiguous(candidates(&["a", "b"]))), 64)]
    #[case::not_email(core(LoginError::NotEmail { raw: "x".into() }), 65)]
    #[case::bad_request(status(400), 65)]
    #[case::index_corrupt(core(LocalDataError::IndexCorrupt { error: None }), 65)]
    #[case::no_input(Error::NoInput { what: "a code" }, 66)]
    #[case::not_found(missing(LookupMiss::not_found()), 66)]
    #[case::not_cached(core(LocalDataError::NotCached { board: "x".into() }), 66)]
    #[case::html_instead_of_json(core(HttpError::HtmlInsteadOfJson { status: 200 }), 69)]
    #[case::other_status(status(418), 69)]
    #[case::hud_refused(core(BoardSocketError::NotSubscribed { code: SocketErrorCode::InternalError }), 75)]
    #[case::already_exists(
        core(LocalDataError::AlreadyExists { path: "board.holst".into() }),
        73
    )]
    #[case::write_stdout(Error::WriteStdout(io::ErrorKind::BrokenPipe.into()), 74)]
    #[case::file_read(
        core(LocalDataError::FileRead { path: "x".into(), error: io::ErrorKind::BrokenPipe.into() }),
        74
    )]
    #[case::server_error(status(503), 75)]
    #[case::hud_timeout(core(BoardSocketError::Timeout { seconds: 30 }), 75)]
    #[case::keychain_timeout(
        core(LocalDataError::Keychain(KeychainError::TimedOut(
            std::time::Duration::ZERO
        ))),
        75
    )]
    #[case::cancelled(Error::Cancelled, 75)]
    #[case::keychain_spawn(
        core(LocalDataError::Keychain(KeychainError::Spawn {
            program: "security".into(),
            error: io::ErrorKind::NotFound.into(),
        })),
        69
    )]
    #[case::unauthorized(status(401), 77)]
    #[case::not_yours(core(BoardChangeError::NotYours { reference: "x".into() }), 77)]
    #[case::unconfirmed(
        Error::Unconfirmed(Unconfirmed::new(
            "deleting a board",
            ChangeRisk::Irreversible
        )),
        77
    )]
    #[case::cache_dir_relative(core(InputError::CacheDirRelative { value: "x".into() }), 78)]
    #[case::no_downloads(Error::NoDownloads, 78)]
    #[case::several_takes_the_first(
        core(InputError::Several { errors: vec![refused(404), refused(401)] }),
        66
    )]
    fn each_error_class_has_its_exit_code(#[case] error: Error, #[case] expected: u8) {
        assert_eq!(code(&error), expected);
        assert!(
            crate::exit::EXIT_CODES
                .iter()
                .any(|&(code, _)| code == expected),
            "{expected} is not in the EXIT CODES block of --help"
        );
    }

    #[rstest]
    #[case::ctrl_c(holst_form::error::FormError::Interrupted, 130)]
    #[case::escape(holst_form::error::FormError::Cancelled, 75)]
    fn a_form_stop_keeps_its_exit_code(
        #[case] stop: holst_form::error::FormError,
        #[case] expected: u8,
    ) {
        assert_eq!(code(&Error::from(stop).shown()), expected);
    }
}
