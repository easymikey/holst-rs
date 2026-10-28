//! Advice under an error, in the words of the surface that shows it.

pub const LOGIN: &str = "holst auth login";
pub const TREE: &str = "holst b ls";
pub const TREE_TRASH: &str = "holst b ls --view trash";
pub const OBJECTS: &str = "holst o ls BOARD";
pub const BACKUPS: &str = "holst backup ls BOARD";
pub const COMMANDS: [&str; 5] = [LOGIN, TREE, TREE_TRASH, OBJECTS, BACKUPS];

/// Advice shown under an error, one action per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, strum::EnumIter)]
pub enum Hint {
    LoginAgain,
    AskOwner,
    CheckId,
    RetryLater,
    CheckRequest,
    ServerFailed,
    WaitAsAsked,
    CheckNetwork,
    CheckBeforeRetry,
    ReadBeforeRetry,
    NarrowDown,
    ChooseId,
    ListBoards,
    ListTree,
    ListTrash,
    ListObjects,
    ListBackups,
    UnlockKeychain,
    UseHome,
    ReplaceStale,
    FullLogin,
    ConfirmFirst,
    LoadOnline,
}

/// Where a hint is shown: the command line names commands, MCP names tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter)]
pub enum Surface {
    Cli,
    Mcp,
}

impl Surface {
    /// How this surface confirms an irreversible action.
    ///
    /// ```
    /// use holst_core::error::hint::Surface;
    ///
    /// assert_eq!(Surface::Cli.confirmation(), "--yes");
    /// assert_eq!(Surface::Mcp.confirmation(), "confirm: true");
    /// ```
    #[must_use]
    pub const fn confirmation(self) -> &'static str {
        match self {
            Surface::Cli => "--yes",
            Surface::Mcp => "confirm: true",
        }
    }
}

/// A hint in the words of one surface: one line in the rustc style.
///
/// ```
/// use holst_core::error::hint::{Hint, Surface};
///
/// assert_eq!(
///     Hint::LoginAgain.on(Surface::Cli).to_string(),
///     "run `holst auth login` to log in again"
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
#[display("{}", hint.text(*surface))]
pub struct Advice {
    pub hint: Hint,
    pub surface: Surface,
}

impl Hint {
    /// This hint as `surface` shows it.
    #[must_use]
    pub const fn on(self, surface: Surface) -> Advice {
        Advice {
            hint: self,
            surface,
        }
    }

    /// The text on `surface`: the CLI names commands, MCP names tools.
    #[must_use]
    pub fn text(self, surface: Surface) -> &'static str {
        match (surface, self.words()) {
            (_, SurfaceText::Same(text))
            | (Surface::Cli, SurfaceText::Each { cli: text, .. })
            | (Surface::Mcp, SurfaceText::Each { mcp: text, .. }) => text,
        }
    }

    const fn words(self) -> SurfaceText {
        match self {
            Hint::LoginAgain => SIGN_IN,
            Hint::AskOwner => SurfaceText::Same("ask the owner for access"),
            Hint::CheckId => CHECK_ID,
            Hint::RetryLater => SurfaceText::Same("try again in a minute"),
            Hint::CheckRequest => CHECK_REQUEST,
            Hint::ServerFailed => SERVER_FAILED,
            Hint::WaitAsAsked => WAIT_AS_ASKED,
            Hint::CheckNetwork => CHECK_NETWORK,
            Hint::CheckBeforeRetry => CHECK_BEFORE_RETRY,
            Hint::ReadBeforeRetry => READ_BEFORE_RETRY,
            Hint::NarrowDown => NARROW_DOWN,
            Hint::ChooseId => PICK_ID,
            Hint::ListBoards => LIST_BOARDS,
            Hint::ListTree => LIST_TREE,
            Hint::ListTrash => LIST_TRASH,
            Hint::ListObjects => LIST_OBJECTS,
            Hint::ListBackups => LIST_BACKUPS,
            Hint::UnlockKeychain => UNLOCK_KEYCHAIN,
            Hint::UseHome => USE_HOME,
            Hint::ReplaceStale => REPLACE_STALE,
            Hint::FullLogin => FULL_LOGIN,
            Hint::ConfirmFirst => CONFIRM_FIRST,
            Hint::LoadOnline => LOAD_ONLINE,
        }
    }
}

const SIGN_IN: SurfaceText = SurfaceText::Each {
    cli: "run `holst auth login` to log in again",
    mcp: "call auth with action login, ask the user to open its link to log in again, then call auth",
};
const CHECK_ID: SurfaceText =
    SurfaceText::Same("check the id or link — it may be deleted or in the trash");
const CHECK_REQUEST: SurfaceText = SurfaceText::Same(
    "check the request's ids and parameters — the server refused them",
);
const SERVER_FAILED: SurfaceText = SurfaceText::Same(
    "check the request's ids and parameters — the server failed on it, and the \
     same request is likely to fail again",
);
const WAIT_AS_ASKED: SurfaceText =
    SurfaceText::Same("wait as long as the error says, then try again");
const CHECK_NETWORK: SurfaceText = SurfaceText::Each {
    cli: "check the network, the proxy and `HOLST_BASE_URL`",
    mcp: "ask the user to check the network, the proxy and HOLST_BASE_URL in the \
          server settings",
};
const CHECK_BEFORE_RETRY: SurfaceText =
    SurfaceText::Same("check whether the change was applied before you retry");
const READ_BEFORE_RETRY: SurfaceText = SurfaceText::Each {
    cli: "run `holst o ls BOARD` to see whether the change is on the \
          board before you retry — a retry may duplicate it",
    mcp: "call the object tool to see whether the change is on the board before \
          you retry — a retry may duplicate it",
};
const NARROW_DOWN: SurfaceText = SurfaceText::Each {
    cli: "give the id, or narrow the search with `--workspace`",
    mcp: "pass an @N or an id, or narrow the search with workspace",
};
const PICK_ID: SurfaceText = SurfaceText::Each {
    cli: "give one of the ids from the list",
    mcp: "pass one of the @N from the list",
};
// A name is never looked up in the trash, so say where else to look.
const LIST_BOARDS: SurfaceText = SurfaceText::Each {
    cli: "run `holst b ls` to see the boards, `holst b ls --view trash` with \
          the deleted ones",
    mcp: "call board action=ls to see the boards, with view=trash for the \
          deleted ones",
};
const LIST_TREE: SurfaceText = SurfaceText::Each {
    cli: "run `holst b ls` to see the boards and folders",
    mcp: "call board action=ls to see the boards and folders",
};
const LIST_TRASH: SurfaceText = SurfaceText::Each {
    cli: "run `holst b ls --view trash` to see what is in the trash",
    mcp: "call board action=ls view=trash to see what is in the trash",
};
const LIST_OBJECTS: SurfaceText = SurfaceText::Each {
    cli: "run `holst o ls BOARD` to see the object ids",
    mcp: "call the object tool to see the object ids",
};
const LIST_BACKUPS: SurfaceText = SurfaceText::Each {
    cli: "run `holst backup ls BOARD` to see the backup keys",
    mcp: "call backup with action ls to see the backup keys",
};
const UNLOCK_KEYCHAIN: SurfaceText = SurfaceText::Each {
    cli: "unlock the keychain and allow access, or keep the login in \
          `HOLST_HOME`",
    mcp: "ask the user to unlock the keychain and allow access, or to set \
          HOLST_HOME in the server settings",
};
const USE_HOME: SurfaceText = SurfaceText::Each {
    cli: "start gnome-keyring or KWallet, or set `HOLST_HOME=<dir>` to keep the \
          login in `<dir>/session`",
    mcp: "ask the user to start gnome-keyring or KWallet, or to set HOLST_HOME \
          in the server settings",
};
const REPLACE_STALE: SurfaceText = SurfaceText::Each {
    cli: "unlock the keychain and run `holst auth login` again, so the new login \
          replaces the older one",
    mcp: "ask the user to unlock the keychain and log in again, so the new \
          login replaces the older one",
};
const FULL_LOGIN: SurfaceText = SurfaceText::Each {
    cli: "run `holst auth login` to get a full session — a bare token has no cookies",
    mcp: "call auth with action login and ask the user to open its link to log in \
          again — a bare token has no cookies",
};
const CONFIRM_FIRST: SurfaceText = SurfaceText::Each {
    cli: "check the target, then run the command again with `--yes`",
    mcp: "ask the user first — it may also affect other people — then call \
          again with confirm: true",
};
const LOAD_ONLINE: SurfaceText = SurfaceText::Each {
    cli: "run the command once without `--offline` to cache the board",
    mcp: "call the object tool once to load the board",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceText {
    Same(&'static str),
    Each {
        cli: &'static str,
        mcp: &'static str,
    },
}

#[cfg(test)]
pub(super) mod decided {
    use std::{collections::BTreeSet, fmt::Debug};

    use strum::IntoEnumIterator;

    use crate::error::hint::Hint;

    pub(in crate::error) fn assert_every_variant<E, D>(
        samples: Vec<(E, Option<Hint>)>,
        hint: impl Fn(&E) -> Option<Hint>,
    ) where
        E: Debug,
        D: IntoEnumIterator + Ord + Debug + for<'a> From<&'a E>,
    {
        let covered: BTreeSet<D> =
            samples.iter().map(|(err, _)| D::from(err)).collect();
        let wrong: Vec<String> = samples
            .into_iter()
            .filter(|(err, expected)| hint(err) != *expected)
            .map(|(err, expected)| {
                format!("{err:?}: {:?}, want {expected:?}", hint(&err))
            })
            .collect();
        assert_eq!(wrong, Vec::<String>::new());
        let missing: Vec<D> =
            D::iter().filter(|kind| !covered.contains(kind)).collect();
        assert_eq!(missing, Vec::<D>::new(), "variants without a hint decision");
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use crate::error::hint::{COMMANDS, Hint, Surface};

    #[test]
    fn every_hint_is_one_rustc_style_line() {
        let bad: Vec<String> = Hint::iter()
            .flat_map(|hint| Surface::iter().map(move |surface| hint.on(surface)))
            .map(|advice| advice.to_string())
            .filter(|text| {
                text.is_empty()
                    || text.contains('\n')
                    || text.ends_with('.')
                    || text.starts_with(|c: char| c.is_uppercase())
            })
            .collect();
        assert_eq!(bad, Vec::<String>::new());
        assert!(Hint::iter().count() >= 11);
    }

    #[test]
    fn cli_hints_name_only_known_commands() {
        let named: Vec<String> = Hint::iter()
            .map(|hint| hint.on(Surface::Cli).to_string())
            .flat_map(|text| {
                text.split('`')
                    .skip(1)
                    .step_by(2)
                    .filter(|span| span.starts_with("holst "))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(!named.is_empty());
        assert!(
            named.iter().all(|cmd| COMMANDS.contains(&cmd.as_str())),
            "{named:?}"
        );
    }

    #[test]
    fn mcp_hints_never_mention_cli_commands_or_flags() {
        let cli: Vec<String> = Hint::iter()
            .map(|hint| hint.on(Surface::Mcp).to_string())
            .filter(|text| {
                text.contains("holst ")
                    || text.contains("--")
                    || text.contains('`')
                    || COMMANDS.iter().any(|command| text.contains(command))
            })
            .collect();
        assert_eq!(cli, Vec::<String>::new());
    }
}
