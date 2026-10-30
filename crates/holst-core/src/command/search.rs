//! `holst search`: names of boards and folders, text on the boards, or both.

use std::{borrow::Borrow, cmp::Reverse, pin::Pin, sync::Arc};

use holst_macros::CommandArgs;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};

use crate::{
    command::{
        board::{
            Caching,
            show::{Loading, cache_use, through_cache},
        },
        board_load::CacheUse,
        board_text,
        lookup::{self, Fresh, IndexedNodes},
        table::{EnumArgument, not_value},
    },
    domain::{
        account::Workspace,
        board::ObjectKind,
        board_text::{
            BoardTextHits,
            BoardTextQuery,
            index::BoardIndex,
            search::{ObjectFilter, TextHit},
        },
        id::BoardId,
        limit::Limit,
        lookup::choice::{
            self,
            BOARDS,
            ChoiceQuery,
            MatchCoverage,
            SkippedWorkspaces,
            TreeReads,
        },
        node::NodeKind,
        tree::name_index::{
            build::NameIndex,
            query::{NodeFilter, WorkspaceNameHit, board_names, name_hits},
        },
    },
    effect::settings::CacheSettings,
    error::{Error, Result},
    remote::client::Client,
};

/// The trees of workspaces that were read, each behind `I`.
type Trees<I> = [(Workspace, I)];

const COMMAND: &str = "search";
/// How many matches `search` keeps when `-l` is left out.
const DEFAULT_LIMIT: usize = 20;

/// What `holst search` looks through.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SearchMode {
    Names,
    BoardText,
    #[default]
    Both,
}

impl EnumArgument for SearchMode {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Search,
    risk = Read,
    about = "Find boards by name and text on boards",
    help = "Find boards by name and text on boards, the best match first.\n\
            It prints one line per match with @N and its link. Words match in any order; case and punctuation do not matter.",
    example = "holst search \"retro\"",
    example = "holst search \"design\" --mode board-text -w easy -k sticker",
    example = "holst search \"Roadmap\" --mode names -l 0"
)]
pub struct SearchArgs {
    #[argument(positional, value_name = "query", help = "Words to find, in any order")]
    pub query: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "mode",
        help = "What to search: names, board-text or both; both by default"
    )]
    pub mode: Option<SearchMode>,
    #[argument(
        value_name = "board",
        help = "Only this board: id, link or part of its name; reads board text"
    )]
    pub board: Option<String>,
    #[argument(
        letter = 'k',
        value_name = "kind",
        help = "Only this kind: board, folder or any for names; sticker, text, shape, frame, arrow, image… for board text",
        tool_doc = "names: board, folder or any; board_text: an object kind."
    )]
    pub kind: Option<String>,
    #[argument(help = "Also look in the trash", tool_doc = "names: in the trash.")]
    pub trash: bool,
    #[argument(
        letter = 'l',
        value_name = "n",
        help = "At most this many matches; 0 shows all; 20 by default"
    )]
    pub limit: Option<u32>,
    #[argument(
        help = "Ignore the cached copy and download the board again",
        surfaces = [Cli]
    )]
    pub refresh: bool,
    #[argument(
        help = "No network: read the text of the --board, an id or link, from the cache, or fail when there is none",
        surfaces = [Cli]
    )]
    pub offline: bool,
}

/// The matches of a search, best first.
#[derive(Debug, Serialize)]
pub struct Found {
    pub query: String,
    pub hits: Vec<SearchHit>,
    /// How many matches there were before the limit, names and text together.
    pub total: usize,
    /// How many boards the text search read.
    pub boards_read: usize,
    /// Workspaces whose boards and folders could not be read, with why.
    pub skipped_workspaces: Vec<String>,
    /// Boards whose text could not be read.
    pub skipped_boards: Vec<SkippedBoard>,
}

/// A board whose text the search could not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedBoard {
    pub id: String,
    pub name: String,
    pub reason: String,
}

/// One match: a name or a piece of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "in", rename_all = "snake_case")]
pub enum SearchHit {
    Name(NameMatch),
    Text(TextMatch),
}

impl SearchHit {
    fn score(&self) -> u32 {
        match self {
            SearchHit::Name(found) => found.score,
            SearchHit::Text(found) => found.score,
        }
    }
}

/// A board or folder whose name matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameMatch {
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    pub workspace: String,
    /// The folders it is in, joined with `/`; empty at the root.
    pub path: String,
    pub in_trash: bool,
    /// Only a board has a link.
    pub link: Option<String>,
    pub score: u32,
}

/// An object whose text matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextMatch {
    pub board: String,
    pub board_name: String,
    pub object: String,
    pub kind: String,
    pub excerpt: String,
    /// Where the match is in the excerpt, as `[from, to)` in characters.
    pub matched: [usize; 2],
    /// The match has a typo.
    pub fuzzy: bool,
    pub link: String,
    pub score: u32,
}

/// A board's text index, read by whoever reads boards for the search.
pub type BoardRead<'a> =
    Pin<Box<dyn Future<Output = Result<Arc<BoardIndex>>> + Send + 'a>>;

/// Where a search reads from: requests, or what a server keeps.
pub trait Reads: Sync {
    /// A tree as the source keeps it.
    type Tree: Borrow<NameIndex> + Send + Sync;

    /// All workspaces of the user.
    fn workspaces(&self) -> impl Future<Output = Result<Vec<Workspace>>> + Send;

    /// The workspace `reference` names, else the only one.
    fn workspace(
        &self,
        reference: Option<&str>,
    ) -> impl Future<Output = Result<Workspace>> + Send {
        async move { choice::workspace(&self.workspaces().await?, reference) }
    }

    /// The live trees or the trashes of `workspaces`; a workspace that cannot
    /// be read is reported, not fatal.
    fn trees(
        &self,
        workspaces: Vec<Workspace>,
        nodes: IndexedNodes,
    ) -> impl Future<Output = Result<TreeReads<Self::Tree>>> + Send;

    /// The text index of one board.
    fn board(&self, board: String) -> BoardRead<'_>;
}

/// Every read is a request, the boards through the cache of `loading`.
struct Requests<'a> {
    client: &'a Client,
    loading: Loading<'a>,
}

impl Reads for Requests<'_> {
    type Tree = NameIndex;

    async fn workspaces(&self) -> Result<Vec<Workspace>> {
        match self.loading.cache_use {
            // Offline there are no workspaces to read the trees of.
            CacheUse::Offline => Ok(Vec::new()),
            CacheUse::Use | CacheUse::Refresh => lookup::workspaces(self.client).await,
        }
    }

    async fn workspace(&self, reference: Option<&str>) -> Result<Workspace> {
        match self.loading.cache_use {
            CacheUse::Offline => choice::workspace(&[], reference),
            CacheUse::Use | CacheUse::Refresh => {
                lookup::workspace(self.client, reference).await
            }
        }
    }

    async fn trees(
        &self,
        workspaces: Vec<Workspace>,
        nodes: IndexedNodes,
    ) -> Result<TreeReads<NameIndex>> {
        let fresh = match self.loading.cache_use {
            CacheUse::Refresh => Fresh::Asked,
            CacheUse::Use | CacheUse::Offline => Fresh::Kept,
        };
        choice::partition_reads(
            lookup::indexes(self.client, workspaces, nodes, fresh).await,
        )
    }

    fn board(&self, board: String) -> BoardRead<'_> {
        Box::pin(async move {
            board_text::load_index(self.client, self.loading, &board)
                .await
                .map(Arc::new)
        })
    }
}

/// Searches names, board text or both, best first; `--refresh` and `--offline`
/// of `args` apply.
///
/// # Errors
/// An argument that does not fit the mode, a workspace or board that is not
/// found or not clear, and fatal errors of the requests; a workspace or board
/// that cannot be read is reported in the result.
pub async fn search(
    client: &Client,
    cache: &CacheSettings,
    args: &SearchArgs,
) -> Result<Found> {
    let caching = Caching {
        refresh: args.refresh,
        offline: args.offline,
    };
    let cache_use = cache_use(&caching, COMMAND)?;
    through_cache(client, cache, cache_use, async |loading| {
        searched(client, loading, args).await
    })
    .await
}

async fn searched(
    client: &Client,
    loading: Loading<'_>,
    args: &SearchArgs,
) -> Result<Found> {
    offline_board(loading.cache_use, args)?;
    search_with(client, args, &Requests { client, loading }).await
}

/// Offline, only a board by id or link is searched; the rest need requests.
fn offline_board(cache_use: CacheUse, args: &SearchArgs) -> Result<()> {
    match (cache_use, args.board.as_deref().and_then(BoardId::find_in)) {
        (CacheUse::Offline, None) => Err(not_value(
            COMMAND,
            "offline",
            "left out, or used with --board and a board id or link",
        )),
        (CacheUse::Offline | CacheUse::Use | CacheUse::Refresh, _) => Ok(()),
    }
}

/// [`search`] with the reads in `reads`; `client` only makes the links.
///
/// # Errors
/// As [`search`].
pub async fn search_with<R: Reads>(
    client: &Client,
    args: &SearchArgs,
    reads: &R,
) -> Result<Found> {
    let plan = Plan::of(args)?;
    let limit = Limit::from(args.limit.map_or(DEFAULT_LIMIT, |count| {
        usize::try_from(count).unwrap_or(usize::MAX)
    }));
    let workspaces = searched_workspaces(reads, args.workspace.as_deref()).await?;
    let (live, unread) = reads.trees(workspaces.clone(), IndexedNodes::Live).await?;
    let (trash, trash_unread) = match plan.names {
        Some(NamesPlan { trash: true, .. }) => {
            reads.trees(workspaces, IndexedNodes::Trash).await?
        }
        Some(NamesPlan { trash: false, .. }) | None => {
            (Vec::new(), SkippedWorkspaces::default())
        }
    };
    let (named, named_total) = plan.names.map_or_else(
        || (Vec::new(), 0),
        |names| name_matches(client, (&live, &trash), names, &args.query, limit),
    );
    let mut texts = Vec::new();
    let mut total = named_total;
    let mut boards_read = 0;
    let mut skipped_boards = Vec::new();
    if let Some(text) = &plan.text {
        let found = text_found(
            (&live, &unread),
            text,
            BoardTextQuery {
                query: &args.query,
                filter: ObjectFilter {
                    board: None,
                    kind: text.kind.clone(),
                },
                limit,
            },
            |board| reads.board(board),
        )
        .await?;
        texts = text_matches(client, &found);
        total += found.total;
        boards_read = found.boards_read;
        skipped_boards = found
            .unread
            .iter()
            .map(|board| SkippedBoard {
                id: board.board.clone(),
                name: found.board_name(&board.board),
                reason: board.reason.clone(),
            })
            .collect();
    }
    Ok(Found {
        query: args.query.clone(),
        hits: merged(named, texts, limit),
        total,
        boards_read,
        skipped_workspaces: unread
            .0
            .iter()
            .chain(&trash_unread.0)
            .map(ToString::to_string)
            .unique()
            .collect(),
        skipped_boards,
    })
}

/// Which halves of the search run, and how.
struct Plan<'a> {
    names: Option<NamesPlan>,
    text: Option<TextPlan<'a>>,
}

#[derive(Debug, Clone, Copy)]
struct NamesPlan {
    kind: Option<NodeKind>,
    trash: bool,
}

#[derive(Debug)]
struct TextPlan<'a> {
    board: Option<&'a str>,
    kind: Option<ObjectKind>,
}

/// What `--kind` names: a kind of node, or a kind of object.
enum KindWord {
    Node(Option<NodeKind>),
    Object(ObjectKind),
}

fn kind_word(raw: Option<&str>) -> Option<KindWord> {
    raw.and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
        "board" => Some(KindWord::Node(Some(NodeKind::Board))),
        "folder" => Some(KindWord::Node(Some(NodeKind::Folder))),
        "any" => Some(KindWord::Node(None)),
        _ => ObjectKind::typed(raw).map(KindWord::Object),
    })
}

impl<'a> Plan<'a> {
    /// A `--board` or an object kind asks for the text, a board or folder
    /// kind for the names; the mode must agree, and with `both` they must not
    /// disagree among themselves.
    fn of(args: &'a SearchArgs) -> Result<Self> {
        let mode = args.mode.unwrap_or_default();
        let board = args
            .board
            .as_deref()
            .map(str::trim)
            .filter(|board| !board.is_empty());
        let (node_kind, object_kind) = match kind_word(args.kind.as_deref()) {
            Some(KindWord::Node(kind)) => (kind, None),
            Some(KindWord::Object(kind)) => (None, Some(kind)),
            None => (None, None),
        };
        let for_text = board.is_some() || object_kind.is_some();
        let for_names = node_kind.is_some();
        let wants_names = mode != SearchMode::BoardText && !for_text;
        let wants_text = mode != SearchMode::Names && !for_names;
        conflict(mode, args.trash, (for_text, for_names), board).map_or(Ok(()), Err)?;
        Ok(Self {
            names: wants_names.then_some(NamesPlan {
                kind: node_kind,
                trash: args.trash,
            }),
            text: wants_text.then_some(TextPlan {
                board,
                kind: object_kind,
            }),
        })
    }
}

/// Why the arguments do not fit the mode, if they do not.
fn conflict(
    mode: SearchMode,
    trash: bool,
    (for_text, for_names): (bool, bool),
    board: Option<&str>,
) -> Option<Error> {
    match mode {
        SearchMode::Names if board.is_some() => Some(not_value(
            COMMAND,
            "board",
            "left out with --mode names; a board is searched for its text",
        )),
        SearchMode::Names if for_text => Some(not_value(
            COMMAND,
            "kind",
            "board, folder or any with --mode names",
        )),
        SearchMode::BoardText if for_names => Some(not_value(
            COMMAND,
            "kind",
            "an object kind (sticker, text, shape, frame…) with --mode board_text",
        )),
        SearchMode::BoardText if trash => Some(not_value(
            COMMAND,
            "trash",
            "left out with --mode board_text; only names look into the trash",
        )),
        SearchMode::Both if for_text && for_names => Some(not_value(
            COMMAND,
            "kind",
            "an object kind when --board is given",
        )),
        SearchMode::Both if for_text && trash => Some(not_value(
            COMMAND,
            "trash",
            "left out with --board or an object kind; only names look into the trash",
        )),
        SearchMode::Names | SearchMode::BoardText | SearchMode::Both => None,
    }
}

/// The workspace asked for, else all of the user's.
async fn searched_workspaces<R: Reads>(
    reads: &R,
    reference: Option<&str>,
) -> Result<Vec<Workspace>> {
    match reference {
        Some(reference) => reads.workspace(Some(reference)).await.map(|one| vec![one]),
        None => reads.workspaces().await,
    }
}

fn name_matches<I: Borrow<NameIndex>>(
    client: &Client,
    (live, trash): (&Trees<I>, &Trees<I>),
    names: NamesPlan,
    query: &str,
    limit: Limit,
) -> (Vec<SearchHit>, usize) {
    let kind = names.kind;
    let (alive, alive_total) = name_hits(
        live,
        query,
        NodeFilter {
            kind,
            in_trash: false,
        },
        limit,
    );
    let (trashed, trashed_total) = name_hits(
        trash,
        query,
        NodeFilter {
            kind,
            in_trash: true,
        },
        limit,
    );
    let hits = alive
        .into_iter()
        .chain(trashed)
        .map(|found| SearchHit::Name(name_match(client, found)))
        .collect();
    (hits, alive_total + trashed_total)
}

fn name_match(
    client: &Client,
    (workspace, index, hit): WorkspaceNameHit<'_>,
) -> NameMatch {
    let kind = index.kind(hit.node);
    let id = index.id(hit.node);
    NameMatch {
        link: (kind == NodeKind::Board).then(|| client.board_link(&id)),
        id,
        name: index.name(hit.node).to_owned(),
        kind,
        workspace: workspace.name.clone(),
        path: index.folder_path(hit.node),
        in_trash: index.in_trash(hit.node),
        score: hit.score,
    }
}

/// The text matches of the live boards of `trees`, the one `--board` names or
/// all; each board is read by `load`.
async fn text_found<I, Load, Loading>(
    (trees, unread): (&Trees<I>, &SkippedWorkspaces),
    text: &TextPlan<'_>,
    query: BoardTextQuery<'_>,
    load: Load,
) -> Result<BoardTextHits>
where
    I: Borrow<NameIndex>,
    Load: Fn(String) -> Loading,
    Loading: Future<Output = Result<Arc<BoardIndex>>>,
{
    let known = board_names(trees);
    let boards = match text.board {
        Some(reference) => {
            let board = BoardId::find_in(reference).map(String::from).map_or_else(
                || {
                    choice::one(
                        trees,
                        unread,
                        ChoiceQuery {
                            reference,
                            filter: BOARDS,
                            matching: MatchCoverage::Partial,
                        },
                    )
                },
                Ok,
            )?;
            let name = known
                .iter()
                .find(|(id, _)| *id == board)
                .map_or_else(|| board.clone(), |(_, name)| name.clone());
            vec![(board, name)]
        }
        None => known,
    };
    board_text::search_text(boards, load, &query).await
}

fn text_matches(client: &Client, found: &BoardTextHits) -> Vec<SearchHit> {
    found
        .hits
        .iter()
        .map(|hit| SearchHit::Text(text_match(client, found, hit)))
        .collect()
}

fn text_match(client: &Client, found: &BoardTextHits, hit: &TextHit) -> TextMatch {
    let (before, matched, _) = hit.excerpt.parts();
    let from = before.chars().count();
    TextMatch {
        board: hit.board.clone(),
        board_name: found.board_name(&hit.board),
        object: hit.object.clone(),
        kind: hit.kind.as_str().to_owned(),
        excerpt: hit.excerpt.text.clone(),
        matched: [from, from + matched.chars().count()],
        fuzzy: hit.fuzzy,
        link: client.base_url().object_link(&hit.board, &hit.object),
        score: hit.score,
    }
}

/// Both lists together, the best score first (names before text when equal),
/// cut to `limit`.
fn merged(
    first: Vec<SearchHit>,
    second: Vec<SearchHit>,
    limit: Limit,
) -> Vec<SearchHit> {
    let mut all: Vec<SearchHit> = first
        .into_iter()
        .chain(second)
        .sorted_by_key(|hit| Reverse(hit.score()))
        .collect();
    if let Some(count) = limit.count() {
        all.truncate(count);
    }
    all
}

#[cfg(test)] mod tests;
