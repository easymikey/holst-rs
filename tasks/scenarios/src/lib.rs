//! Scenarios that run on the CLI and the MCP server against one fake Holst.

#![expect(
    clippy::disallowed_methods,
    reason = "the scenarios drive the real binary against real temp directories and the real environment"
)]

mod board_tag;
mod cli;
mod comments;
mod directory;
mod edit;
mod error;
mod export;
mod files;
mod given;
mod holst;
mod mcp;
mod observation;
mod read;
mod session;
mod settings;
mod sharing;
mod storage;
mod surface;
mod templates;
mod then;
mod tree_change;
mod tree_list;
mod whole;
mod workspaces;

pub use holst_core::fake::{FixtureObject, FixtureThread, ThreadAnchor};

pub use crate::{
    board_tag::TagEdit,
    comments::{CommentCommand, CommentsThen, ThreadPin, comments_of, read_time_of},
    edit::{BoardThen, ItemsFile, ObjectCall, board_of},
    error::ScenarioError,
    export::{Export, FileThen, Import, SavedThen, exported_file, saved_in},
    files::{downloaded, downloaded_bytes, objects_on, uploads_of},
    given::{Given, given},
    holst::{BoardReference, Holst, On},
    observation::{
        CountedWorkspace,
        ErrorExit,
        ListedMatch,
        ListedName,
        ListedObject,
        ListedTextHit,
        ListedTree,
        Observation,
        PictureFormat,
    },
    read::Search,
    settings::{CoverEdit, NoteEdit, cover_of, note_of},
    sharing::{
        RightsChange,
        id_of,
        not_owned_by,
        owned_by,
        role_on,
        sharing_of,
        withhold_copying,
        withhold_deleting,
        withhold_hiding,
        withhold_locking,
    },
    storage::{Cleaning, kept_in},
    surface::Surface,
    templates::{
        TemplateAudience,
        TemplateCommand,
        TemplatePart,
        TemplateThen,
        TemplatesThen,
        ThumbnailsThen,
        template_of,
        templates_of,
        thumbnails_of,
        workspace_templates_of,
    },
    then::{FavoritesThen, Then, TreeThen, favorites_of, then, trash_of, tree_of},
    tree_change::{CopyAs, NewParent, TreeCommand},
    tree_list::Ls,
    whole::{DownloadsThen, Look, PictureThen, ShownBoard, downloads_of, picture},
    workspaces::WorkspaceCommand,
};
