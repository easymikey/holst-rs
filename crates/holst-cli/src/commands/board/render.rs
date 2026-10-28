//! What the board tree commands print.

use holst_core::{
    command::{
        board::{
            rights::{Settled, Shared},
            settings::Edited,
            tree::{ListedNode, Listing},
            tree_change::Changed,
        },
        change::{ChangeReport, PlannedRequest},
    },
    domain::{
        node::NodeKind,
        text::{EMPTY, joined, printable},
    },
};

use crate::{
    error::Result,
    output::{self, OutputFormat},
};

/// Prints a page of boards and folders; `link` opens a board.
pub(super) fn listing(
    format: OutputFormat,
    listing: &Listing,
    link: impl Fn(&str) -> String,
) -> Result<()> {
    output::print(format, &listing_text(listing, link), listing)
}

/// Prints what a change did, or what it would do.
pub(super) fn changed(format: OutputFormat, changed: &Changed) -> Result<()> {
    output::print(format, &changed_text(changed), changed)
}

/// Prints what a rights change did, or what it would do.
pub(super) fn shared(format: OutputFormat, shared: &Shared) -> Result<()> {
    output::print(format, &shared_text(shared), shared)
}

/// Prints what a board edit did, or would do.
pub(super) fn edited(format: OutputFormat, edited: &Edited) -> Result<()> {
    output::print(format, &edited_text(edited), edited)
}

fn edited_text(edited: &Edited) -> String {
    let sent = matches!(edited.outcome, ChangeReport::Sent);
    let link = sent.then(|| edited.link.clone());
    joined([edited.phrase()].into_iter().chain(link), "\n").to_string()
}

fn listing_text(listing: &Listing, link: impl Fn(&str) -> String) -> String {
    if listing.nodes.is_empty() {
        EMPTY.to_owned()
    } else {
        page_text(listing, &link)
    }
}

fn page_text(listing: &Listing, link: &impl Fn(&str) -> String) -> String {
    let several = listing.workspaces.len() > 1;
    let before = std::iter::once(None)
        .chain(listing.nodes.iter().map(|node| node.workspace.as_deref()));
    let lines = listing.nodes.iter().zip(before).flat_map(|(node, before)| {
        let heading = node
            .workspace
            .as_deref()
            .filter(|current| several && Some(*current) != before)
            .and_then(|current| workspace_heading(listing, current));
        heading.into_iter().chain([node_line(node, link)])
    });
    let more = listing
        .cursor
        .as_deref()
        .map(|cursor| format!("more: --cursor {cursor}"));
    joined(lines.chain(more), "\n").to_string()
}

fn workspace_heading(listing: &Listing, id: &str) -> Option<String> {
    listing
        .workspaces
        .iter()
        .find(|workspace| workspace.id == id)
        .map(|workspace| format!("{} ({})", printable(&workspace.name), workspace.id))
}

fn node_line(node: &ListedNode, link: &impl Fn(&str) -> String) -> String {
    let name = if node.name.is_empty() {
        "(no name)".into()
    } else {
        printable(&node.name)
    };
    let (slash, place) = match node.kind {
        kind if kind == NodeKind::Folder.as_str() => ("/", node.id.clone()),
        kind if kind == NodeKind::Board.as_str() => ("", link(&node.id)),
        _ => ("", node.id.clone()),
    };
    let trash = if node.in_trash { "  [in trash]" } else { "" };
    format!(
        "{:indent$}{name}{slash}  {place}{trash}",
        "",
        indent = node.depth.saturating_mul(2)
    )
}

fn changed_text(changed: &Changed) -> String {
    let node = &changed.node;
    let (would, request) = match &changed.outcome {
        ChangeReport::Sent => (false, None),
        ChangeReport::DryRun { request } | ChangeReport::Held { request, .. } => {
            (true, Some(request))
        }
    };
    // A board or a folder opens at its link.
    let place = node.link.clone().filter(|_| !would);
    joined(
        [node.phrase(would)]
            .into_iter()
            .chain(request.into_iter().flat_map(PlannedRequest::lines))
            .chain(place),
        "\n",
    )
    .to_string()
}

fn shared_text(shared: &Shared) -> String {
    let place = matches!(shared.outcome, Settled::Carried(ChangeReport::Sent))
        .then(|| shared.link.clone());
    joined(
        [shared.phrase()]
            .into_iter()
            .chain(shared.plan_lines())
            .chain(place),
        "\n",
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::{
            board::{
                ListView,
                tree::{ListedNode, ListedWorkspace, Listing},
                tree_change::{Action, Changed, ChangedNode},
            },
            change::{ChangeReport, PlannedRequest},
        },
        effect::request::QueryParameter,
    };
    use serde_json::json;

    use crate::commands::board::render::{changed_text, listing_text};

    fn node(
        depth: usize,
        kind: &'static str,
        name: &str,
        workspace: &str,
    ) -> ListedNode {
        ListedNode {
            workspace: Some(workspace.to_owned()),
            id: format!("id-{name}"),
            name: name.to_owned(),
            kind,
            depth,
            in_trash: false,
        }
    }

    fn workspace(id: &str) -> ListedWorkspace {
        ListedWorkspace {
            id: id.to_owned(),
            name: id.to_uppercase(),
        }
    }

    fn listing(
        workspaces: &[&str],
        nodes: Vec<ListedNode>,
        cursor: Option<&str>,
    ) -> Listing {
        Listing {
            view: ListView::Tree,
            workspaces: workspaces.iter().map(|id| workspace(id)).collect(),
            total: nodes.len(),
            nodes,
            cursor: cursor.map(str::to_owned),
        }
    }

    fn link(id: &str) -> String {
        format!("https://holst/{id}")
    }

    #[test]
    fn a_tree_is_indented_with_folders_ending_in_a_slash() {
        let page = listing(
            &["team"],
            vec![
                node(0, "folder", "Work", "team"),
                node(1, "board", "Plan", "team"),
                node(0, "board", "", "team"),
            ],
            None,
        );
        assert_eq!(
            listing_text(&page, link),
            "Work/  id-Work\n  Plan  https://holst/id-Plan\n(no name)  https://holst/id-"
        );
    }

    #[test]
    fn several_workspaces_get_a_heading_and_a_page_names_its_cursor() {
        let page = listing(
            &["team", "ops"],
            vec![node(0, "board", "A", "team"), node(0, "board", "B", "ops")],
            Some("2"),
        );
        assert_eq!(
            listing_text(&page, link),
            "TEAM (team)\nA  https://holst/id-A\nOPS (ops)\nB  https://holst/id-B\nmore: --cursor 2"
        );
        assert_eq!(
            listing_text(&listing(&["team"], vec![], None), link),
            "(empty)"
        );
    }

    fn change(action: Action, outcome: ChangeReport) -> Changed {
        Changed {
            node: ChangedNode {
                action,
                id: Some("b1".to_owned()),
                name: Some("Plan".to_owned()),
                previous_name: Some("Old".to_owned()),
                kind: Some("board"),
                link: Some("https://holst/b1".to_owned()),
                boards_along: 0,
                left_behind: Vec::new(),
                left_in: None,
            },
            outcome,
        }
    }

    fn request() -> PlannedRequest {
        PlannedRequest {
            method: "POST",
            path: "/gapi/channel/b1/trash".to_owned(),
            query: vec![QueryParameter {
                name: "a".to_owned(),
                value: "1".to_owned(),
            }],
            body: Some(json!({ "x": 1 })),
        }
    }

    #[test]
    fn a_sent_change_says_what_happened_and_where_to_find_it() {
        assert_eq!(
            changed_text(&change(Action::Create, ChangeReport::Sent)),
            "created board Plan\nhttps://holst/b1"
        );
        assert_eq!(
            changed_text(&change(Action::Rename, ChangeReport::Sent)),
            "renamed Old to Plan\nhttps://holst/b1"
        );
    }

    #[test]
    fn a_created_folder_shows_its_link_not_its_id() {
        let board = change(Action::Create, ChangeReport::Sent);
        let folder = Changed {
            node: ChangedNode {
                kind: Some("folder"),
                link: Some("https://holst/w/w1/f/b1".to_owned()),
                ..board.node
            },
            ..board
        };
        assert_eq!(
            changed_text(&folder),
            "created folder Plan\nhttps://holst/w/w1/f/b1"
        );
    }

    #[test]
    fn a_dry_run_says_would_and_shows_the_request() {
        let planned = change(Action::Move, ChangeReport::DryRun { request: request() });
        let dry = Changed {
            node: ChangedNode {
                boards_along: 2,
                ..planned.node
            },
            ..planned
        };
        assert_eq!(
            changed_text(&dry),
            "would move Plan, with 2 boards\nPOST /gapi/channel/b1/trash?a=1\n{\"x\":1}"
        );
    }
}
