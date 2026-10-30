//! The `search` tool: the core's search, and what it says of the matches.

use std::sync::Arc;

use holst_core::{
    command::{
        lookup::IndexedNodes,
        search::{
            self,
            BoardRead,
            Found,
            NameMatch,
            Reads,
            SearchArgs,
            SearchHit,
            TextMatch,
        },
    },
    domain::{
        account::Workspace,
        lookup::choice::TreeReads,
        node::NodeKind,
        text::printable,
        tree::name_index::build::NameIndex,
    },
};
use itertools::Itertools as _;
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::HandedClient,
    server::McpServer,
};

/// The most the lines of matches take in a reply, in bytes; the rest is cut.
const REPLY_BYTES: usize = 6_000;

/// Reads through what the server keeps, so a second search costs no request.
struct Kept<'a> {
    holst: &'a McpServer,
    client: &'a HandedClient,
}

impl Reads for Kept<'_> {
    type Tree = Arc<NameIndex>;

    async fn workspaces(&self) -> holst_core::Result<Vec<Workspace>> {
        Ok(self.holst.workspaces(self.client).await?.to_vec())
    }

    async fn trees(
        &self,
        workspaces: Vec<Workspace>,
        nodes: IndexedNodes,
    ) -> holst_core::Result<TreeReads<Arc<NameIndex>>> {
        match nodes {
            IndexedNodes::Live => self.holst.trees(self.client, workspaces).await,
            // The tree has no trashed nodes: the trash is a request of its own.
            IndexedNodes::Trash => {
                self.holst.trash_trees(self.client, workspaces).await
            }
        }
    }

    fn board(&self, board: String) -> BoardRead {
        let (holst, client) = (self.holst.clone(), self.client.clone());
        Box::pin(async move { holst.kept_index(&client, &board).await })
    }
}

impl McpServer {
    pub(super) async fn search(&self, arguments: Map<String, Value>) -> CallToolResult {
        self.parsed(arguments, async |args: SearchArgs| match args {
            args if args.query.trim().is_empty() => self.fail(&Error::EmptyQuery),
            args => {
                self.run(async |client| self.found(&client, args).await)
                    .await
            }
        })
        .await
    }

    async fn found(&self, client: &HandedClient, args: SearchArgs) -> Result<String> {
        let reads = Kept {
            holst: self,
            client,
        };
        let found = search::search_with(client, &args, &reads).await?;
        Ok(self.found_text(&found))
    }

    /// One line per match, best first, each with its `@N`.
    pub(crate) fn found_text(&self, found: &Found) -> String {
        if found.hits.is_empty() {
            format!("no matches for \"{}\"\n{}", found.query, unread(found))
        } else {
            self.listed(found)
        }
    }

    fn listed(&self, found: &Found) -> String {
        let read = match found.boards_read {
            0 => String::new(),
            boards => format!(" (boards read: {boards})"),
        };
        let several = found
            .hits
            .iter()
            .filter_map(|hit| match hit {
                SearchHit::Name(name) => Some(name.workspace.as_str()),
                SearchHit::Text(_) => None,
            })
            .unique()
            .count()
            > 1;
        let mut bytes = 0;
        let lines = found
            .hits
            .iter()
            .map(|hit| match hit {
                SearchHit::Name(name) => self.name_line(name, several),
                SearchHit::Text(text) => self.text_line(text),
            })
            // At least one line, whatever its size.
            .take_while(|line| {
                let fits = bytes == 0 || bytes + line.len() <= REPLY_BYTES;
                bytes += line.len();
                fits
            })
            .collect::<Vec<_>>();
        let cut = match found.hits.len() - lines.len() {
            0 => String::new(),
            more => format!(
                "… {more} more matches over the reply budget: narrow the search with workspace, board, kind or mode\n"
            ),
        };
        let head = format!(
            "\"{}\": {} of {}{read}\n",
            found.query,
            lines.len(),
            found.total
        );
        format!("{head}{}{cut}{}", lines.join(""), unread(found))
    }

    fn name_line(&self, found: &NameMatch, with_workspace: bool) -> String {
        let slash = match found.kind {
            NodeKind::Folder => "/",
            NodeKind::Board | NodeKind::Other => "",
        };
        let place = [
            with_workspace.then_some(found.workspace.as_str()),
            Some(found.path.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .join("/");
        let place = if place.is_empty() {
            place
        } else {
            format!(" · {place}")
        };
        let trash = if found.in_trash { " (trash)" } else { "" };
        format!(
            "{} {}{slash}{place}{trash}\n",
            self.state.alias(&found.id),
            printable(&found.name)
        )
    }

    fn text_line(&self, found: &TextMatch) -> String {
        let typo = if found.fuzzy { "~" } else { "" };
        format!(
            "{} {} in {} {}: {typo}{}\n",
            self.state.alias(&found.object),
            found.kind,
            self.state.alias(&found.board),
            printable(&found.board_name),
            printable(&found.excerpt)
        )
    }
}

/// What the search could not read.
fn unread(found: &Found) -> String {
    let workspaces = if found.skipped_workspaces.is_empty() {
        String::new()
    } else {
        format!("skipped: {}\n", found.skipped_workspaces.join("; "))
    };
    let boards = if found.skipped_boards.is_empty() {
        String::new()
    } else {
        format!(
            "boards not read: {}\n",
            found
                .skipped_boards
                .iter()
                .map(|board| format!("{}: {}", printable(&board.name), board.reason))
                .join("; ")
        )
    };
    workspaces + &boards
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::search::{Found, NameMatch, SearchHit, SkippedBoard, TextMatch},
        domain::node::NodeKind,
    };

    use crate::server::McpServer;

    fn name(id: &str, name: &str, workspace: &str, path: &str) -> NameMatch {
        NameMatch {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: NodeKind::Board,
            workspace: workspace.to_owned(),
            path: path.to_owned(),
            in_trash: false,
            link: None,
            score: 100,
        }
    }

    fn found(hits: Vec<SearchHit>, total: usize) -> Found {
        Found {
            query: "plan".to_owned(),
            hits,
            total,
            boards_read: 0,
            skipped_workspaces: Vec::new(),
            skipped_boards: Vec::new(),
        }
    }

    #[test]
    fn names_say_their_workspace_only_across_several() {
        let holst = McpServer::offline();
        let one = found(vec![SearchHit::Name(name("b1", "Plan", "easy", "Q4"))], 1);
        let many = found(
            vec![
                SearchHit::Name(name("b1", "Plan", "easy", "")),
                SearchHit::Name(NameMatch {
                    kind: NodeKind::Folder,
                    in_trash: true,
                    ..name("f2", "Plans", "team", "Q4")
                }),
            ],
            5,
        );
        assert_eq!(
            [holst.found_text(&one), holst.found_text(&many)],
            [
                "\"plan\": 1 of 1\n@1 Plan · Q4\n",
                "\"plan\": 2 of 5\n@1 Plan · easy\n@2 Plans/ · team/Q4 (trash)\n"
            ]
        );
    }

    #[test]
    fn text_hits_name_the_object_and_the_board() {
        let holst = McpServer::offline();
        let hit = SearchHit::Text(TextMatch {
            board: "b1".to_owned(),
            board_name: "Roadmap".to_owned(),
            object: "o1".to_owned(),
            kind: "sticker".to_owned(),
            excerpt: "the plan".to_owned(),
            matched: [4, 8],
            fuzzy: true,
            link: "https://x".to_owned(),
            score: 650,
        });
        let mut found = found(vec![hit], 1);
        found.boards_read = 3;
        found.skipped_boards = vec![SkippedBoard {
            id: "b2".to_owned(),
            name: "Old".to_owned(),
            reason: "refused".to_owned(),
        }];
        found.skipped_workspaces = vec!["team: not found".to_owned()];
        assert_eq!(
            holst.found_text(&found),
            "\"plan\": 1 of 1 (boards read: 3)\n\
             @1 sticker in @2 Roadmap: ~the plan\n\
             skipped: team: not found\n\
             boards not read: Old: refused\n"
        );
    }

    #[test]
    fn hostile_server_text_is_printable() {
        let holst = McpServer::offline();
        let hit = SearchHit::Text(TextMatch {
            board: "b1".to_owned(),
            board_name: "Road\u{202e}map".to_owned(),
            object: "o1".to_owned(),
            kind: "sticker".to_owned(),
            excerpt: "the \u{1b}[2Jplan".to_owned(),
            matched: [4, 8],
            fuzzy: false,
            link: "https://x".to_owned(),
            score: 650,
        });
        let mut found = found(
            vec![
                hit,
                SearchHit::Name(name("b3", "Pl\u{202e}an", "easy", "Q4")),
            ],
            2,
        );
        found.skipped_boards = vec![SkippedBoard {
            id: "b2".to_owned(),
            name: "Ol\u{1b}d".to_owned(),
            reason: "refused".to_owned(),
        }];
        assert_eq!(
            holst.found_text(&found),
            "\"plan\": 2 of 2\n\
             @1 sticker in @2 Road\u{fffd}map: the \u{fffd}[2Jplan\n\
             @3 Pl\u{fffd}an · Q4\n\
             boards not read: Ol\u{fffd}d: refused\n"
        );
    }

    #[test]
    fn no_hits_say_so() {
        let holst = McpServer::offline();
        assert_eq!(
            holst.found_text(&found(Vec::new(), 0)),
            "no matches for \"plan\"\n"
        );
    }

    #[test]
    fn every_match_is_cut_at_the_reply_budget() {
        let holst = McpServer::offline();
        let hits = (0..500)
            .map(|number| {
                SearchHit::Name(name(
                    &format!("b{number}"),
                    "Plan with a long enough name",
                    "easy",
                    "Q4",
                ))
            })
            .collect();
        let reply = holst.found_text(&found(hits, 500));
        assert!(
            reply.len() < super::REPLY_BYTES + 400,
            "{} bytes",
            reply.len()
        );
        let (head, _) = reply.split_once('\n').unwrap_or_default();
        assert!(
            head.ends_with(" of 500") && !head.starts_with("\"plan\": 500"),
            "{head}"
        );
        assert!(
            reply.contains(" more matches over the reply budget: narrow the search"),
            "{reply}"
        );
    }
}
