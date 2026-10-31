//! The name of a command argument, as the user types it.

use std::fmt;

use strum::{EnumIter, IntoStaticStr};

/// An argument of some command, one variant per argument name.
///
/// Displays as the long flag, `--note-format`, on both surfaces. A name that is
/// positional in one command and a flag in another (`board`, `name`, `text`)
/// shows with the dashes in both, because a name alone cannot tell which.
/// `#[derive(CommandArgs)]` takes `Argument::flag` from the field's name, so
/// a field whose name is not a variant here does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub enum Flag {
    All,
    Assignee,
    At,
    Back,
    Background,
    Bend,
    Board,
    Body,
    Cache,
    Category,
    Cell,
    Check,
    Choice,
    Clear,
    Color,
    Column,
    Columns,
    Confirm,
    Contains,
    Copy,
    Cover,
    Crop,
    Cursor,
    Delete,
    DeleteColumn,
    DeleteRow,
    Description,
    Dir,
    Direction,
    Display,
    Distribute,
    DryRun,
    Due,
    Email,
    Embed,
    Extract,
    Faces,
    File,
    Flip,
    FlipImage,
    Folder,
    For,
    Frame,
    From,
    Heads,
    HideAuthor,
    HideChildren,
    HideContent,
    HideDetails,
    Highlighter,
    Index,
    InsertColumn,
    InsertRow,
    Into,
    Items,
    Joinable,
    Kanban,
    Key,
    Kind,
    Label,
    Language,
    Limit,
    Line,
    Link,
    LinkTo,
    LocalOnly,
    Lock,
    Mark,
    MaxSide,
    Members,
    Merge,
    Mode,
    Name,
    NoCompress,
    NoPreview,
    NoSave,
    Note,
    NoteFormat,
    NoteTemplate,
    Object,
    ObjectList,
    Offline,
    Offset,
    On,
    Only,
    Opacity,
    Order,
    Padding,
    Param,
    Parent,
    Path,
    Permanently,
    PictureFormat,
    Point,
    Provider,
    Query,
    Radius,
    Ratio,
    Reaction,
    Rectangle,
    Refresh,
    Remove,
    Reopen,
    Reply,
    Right,
    Rights,
    Roll,
    Rotate,
    Rows,
    Scale,
    Shape,
    ShowAuthor,
    ShowChildren,
    ShowContent,
    ShowDetails,
    Side,
    Size,
    Start,
    StartPage,
    Status,
    StrokeColor,
    StrokeStyle,
    StrokeWidth,
    Swimlane,
    Tag,
    Template,
    Text,
    TextBackground,
    TextColor,
    TextFormat,
    TextSize,
    Theme,
    Thread,
    Time,
    To,
    Tools,
    Trash,
    Uncheck,
    Unmerge,
    Url,
    User,
    View,
    Weight,
    Width,
    With,
    WithBoards,
    Workspace,
}

impl fmt::Display for Flag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "--{}", <&'static str>::from(self))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use strum::IntoEnumIterator as _;

    use crate::{command::table::CommandName, error::flag::Flag};

    #[test]
    fn every_argument_has_the_flag_of_its_name() {
        let wrong: Vec<_> = CommandName::iter()
            .flat_map(|name| name.command().arguments)
            .filter(|argument| {
                argument.flag.to_string()
                    != format!("--{}", argument.name.replace('_', "-"))
            })
            .map(|argument| argument.name)
            .collect();
        assert!(wrong.is_empty(), "{wrong:?}");
    }

    #[test]
    fn every_flag_is_the_flag_of_some_argument() {
        let used: BTreeSet<String> = CommandName::iter()
            .flat_map(|name| name.command().arguments)
            .map(|argument| argument.flag.to_string())
            .collect();
        let unused: Vec<_> = Flag::iter()
            .map(|flag| flag.to_string())
            .filter(|flag| !used.contains(flag))
            .collect();
        assert!(unused.is_empty(), "{unused:?}");
    }

    #[test]
    fn a_flag_shows_with_two_dashes_and_a_kebab_name() {
        assert_eq!(Flag::NoteFormat.to_string(), "--note-format");
    }
}
