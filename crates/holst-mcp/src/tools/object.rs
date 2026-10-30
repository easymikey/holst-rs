//! The `object` and `object_write` tools: the objects on a board.

use std::path::Path;

use holst_core::{
    command::{
        board::picture::Delivery,
        object::{
            EditArgs,
            GetArgs,
            LsArgs,
            ObjectVerb,
            RemoveArgs,
            add,
            align,
            change::{self, Removed},
            copy,
            get::{self, Downloaded},
            group,
            list::{self, Listing},
            lock,
        },
        table::{Changes, CommandName, parse},
    },
    domain::{
        change::receipt::ChangeReport,
        text::{counted, size_text},
    },
    effect::settings::CacheSettings,
    remote::client::Client,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    memory::alias::expand_json,
    reply::change::written,
    server::McpServer,
    tools::{
        files::{checked_download, checked_upload},
        output::Output,
    },
};

/// The most items one call holds.
const ITEMS_MAX: usize = 100;
/// The objects a text listing shows when the call gives no limit.
const LIST_DEFAULT: u32 = 100;
/// The same for `output: json`: 100 objects are 39 kB, over the reply limit.
const LIST_DEFAULT_JSON: u32 = 25;
/// The most objects a listing shows, also for a limit of 0, which asks for all.
const LIST_MAX: u32 = 1_000;
const TEXT_MAX_CHARS: usize = 120;
/// The arguments that name a board, a frame or an object.
const REFERENCES: [&str; 11] = [
    "board",
    "workspace",
    "frame",
    "object",
    "from",
    "to",
    "into",
    "link_to",
    "parent",
    "kanban",
    "on",
];

impl McpServer {
    pub(super) async fn object_command(
        &self,
        verb: ObjectVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self.object_arguments(verb, arguments) {
            Ok(arguments) => self.run_object_verb(verb, arguments).await,
            Err(err) => self.fail(&err),
        }
    }

    /// The arguments with `@N` expanded in the call and in each of its items,
    /// after the cap on the items.
    fn object_arguments(
        &self,
        verb: ObjectVerb,
        mut arguments: Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        match arguments.get_mut("items").and_then(Value::as_array_mut) {
            Some(items) if items.len() > ITEMS_MAX => Err(Error::TooManyItems {
                command: verb.command().path(),
                max: ITEMS_MAX,
                given: items.len(),
            }),
            Some(items) => items
                .iter_mut()
                .filter_map(Value::as_object_mut)
                .try_for_each(|item| self.expand_references_of(item)),
            None => Ok(()),
        }?;
        self.expand_references_of(&mut arguments)?;
        if verb == ObjectVerb::Add {
            self.check_uploads(&arguments)?;
        }
        Ok(arguments)
    }

    /// Every local `path` of an `add`, of the call and of its items, is read
    /// the way a cover is: not from a secret place.
    fn check_uploads(&self, arguments: &Map<String, Value>) -> Result<()> {
        let items = arguments
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object);
        std::iter::once(arguments)
            .chain(items)
            .filter_map(|given| given.get("path").and_then(Value::as_str))
            .try_for_each(|path| checked_upload(Path::new(path), &self.secrets))
    }

    fn expand_references_of(&self, arguments: &mut Map<String, Value>) -> Result<()> {
        arguments
            .iter_mut()
            .filter(|(name, _)| REFERENCES.contains(&name.as_str()))
            .try_for_each(|(_, value)| expand_json(value, &|text| self.expand(text)))
    }

    async fn run_object_verb(
        &self,
        verb: ObjectVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            ObjectVerb::Ls => self.list_objects(arguments).await,
            ObjectVerb::Add => self.change_objects(arguments, add::add).await,
            ObjectVerb::Edit => {
                let edit = async |client: &Client, args: &EditArgs| {
                    change::edit(client, &self.cache, args).await
                };
                self.change(arguments, edit, Self::removed_text).await
            }
            ObjectVerb::Cp => self.change_objects(arguments, copy::cp).await,
            ObjectVerb::Group => self.change_objects(arguments, group::group).await,
            ObjectVerb::Ungroup => self.change_objects(arguments, group::ungroup).await,
            ObjectVerb::Lock => self.change_objects(arguments, lock::lock).await,
            ObjectVerb::Unlock => self.change_objects(arguments, lock::unlock).await,
            ObjectVerb::Align => self.change_objects(arguments, align::align).await,
            ObjectVerb::Rm => {
                let rm = async |client: &Client, args: &RemoveArgs| {
                    change::rm(client, &self.cache, args).await
                };
                self.change(arguments, rm, Self::removed_text).await
            }
            ObjectVerb::Get => self.download_objects(arguments).await,
        }
    }

    async fn list_objects(&self, mut arguments: Map<String, Value>) -> CallToolResult {
        let taken =
            super::output::take(&mut arguments, CommandName::Object(ObjectVerb::Ls));
        let read = taken.and_then(|output| Ok((output, parse::<LsArgs>(arguments)?)));
        match read {
            Ok((output, args)) => {
                let args = LsArgs {
                    limit: Some(listed_limit(args.limit, output)),
                    ..args
                };
                self.run(async |client| {
                    let listing = list::ls(&client, &self.cache, &args).await?;
                    output.reply(&listing, || self.object_listing_text(&listing))
                })
                .await
            }
            Err(err) => self.fail(&err),
        }
    }

    async fn change_objects<A: Changes>(
        &self,
        arguments: Map<String, Value>,
        make: impl AsyncFnOnce(
            &Client,
            &CacheSettings,
            &A,
        ) -> holst_core::Result<ChangeReport>,
    ) -> CallToolResult {
        let make =
            async |client: &Client, args: &A| make(client, &self.cache, args).await;
        self.change(arguments, make, Self::written_text).await
    }

    async fn download_objects(&self, arguments: Map<String, Value>) -> CallToolResult {
        self.parsed(arguments, async |args: GetArgs| {
            match self.download_place(&args) {
                Ok(()) => {
                    self.run(async |client| {
                        let downloaded = get::get(
                            &client,
                            &self.cache,
                            self.downloads.as_deref(),
                            &args,
                        )
                        .await?;
                        Ok(self.downloaded_text(&downloaded))
                    })
                    .await
                }
                Err(err) => self.fail(&err),
            }
        })
        .await
    }

    /// Refuses standard output and the secret places before any request.
    fn download_place(&self, args: &GetArgs) -> Result<()> {
        let file = args.file.as_ref().map(|file| file.0.as_path());
        match file {
            Some(file) if file == Path::new("-") => Err(Error::NoStdout),
            _ => {
                let dir = args
                    .dir
                    .as_deref()
                    .map(Path::new)
                    .or(self.downloads.as_deref());
                checked_download(file, dir, &self.secrets)
            }
        }
    }

    fn written_text(&self, report: &ChangeReport) -> String {
        written(report, &|id| self.state.alias(id))
    }

    fn removed_text(&self, removed: &Removed) -> String {
        self.written_text(&removed.report)
    }

    fn downloaded_text(&self, downloaded: &Downloaded) -> String {
        let saved =
            downloaded
                .objects
                .iter()
                .filter_map(|object| match &object.delivery {
                    Delivery::Written { file, bytes } => Some(format!(
                        "{} saved {} to {}\n",
                        self.state.alias(&object.object),
                        size_text(*bytes),
                        file.display()
                    )),
                    Delivery::Inline { .. } => None,
                });
        let failed = downloaded
            .not_saved
            .iter()
            .map(|line| format!("not saved: {line}\n"));
        saved.chain(failed).collect()
    }

    pub(crate) fn object_listing_text(&self, listing: &Listing) -> String {
        let rectangle = listing.rectangle.map(|rect| format!("rectangle {rect}\n"));
        let head = format!(
            "{} · {} · {}\n",
            counted(listing.matched, "object"),
            listing.edit_right.as_str(),
            listing.link
        );
        let lines = listing.objects.iter().map(|listed| {
            listed.line(|id| self.state.alias(id), TEXT_MAX_CHARS) + "\n"
        });
        let more = listing.next.map(|next| {
            format!(
                "… {}–{next} of {}; next: offset {next}\n",
                listing.from.saturating_add(1),
                listing.matched
            )
        });
        rectangle
            .into_iter()
            .chain([head])
            .chain(lines)
            .chain(more)
            .collect()
    }
}

/// The limit of a listing: 100 by default (25 as JSON), never more than 1 000.
fn listed_limit(given: Option<u32>, output: Output) -> u32 {
    match given {
        None => match output {
            Output::Text => LIST_DEFAULT,
            Output::Json => LIST_DEFAULT_JSON,
        },
        Some(0) => LIST_MAX,
        Some(limit) => limit.min(LIST_MAX),
    }
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::{
            board::picture::Delivery,
            object::{
                ObjectVerb,
                get::{Downloaded, DownloadedObject},
                list::{ListedObject, Listing, TextStyle},
            },
        },
        domain::board::{EditRight, Object, ObjectKind},
    };
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::{LIST_DEFAULT, LIST_DEFAULT_JSON, LIST_MAX, listed_limit};
    use crate::{
        login::fake::CannedResponse,
        server::McpServer,
        tools::output::Output,
    };

    fn one_object_listing(matched: usize) -> Listing {
        let object = Object {
            id: "a1".into(),
            kind: ObjectKind::Sticker,
            parent: None,
            x: 100.0,
            y: 50.0,
            width: Some(192.0),
            height: Some(192.0),
            z_index: 0.0,
            text: String::new(),
            color: None,
            look: None,
            media: None,
            ends: None,
            author: None,
            locked: false,
            rotation: 0.0,
            link_to: None,
            file: None,
            fields: Value::Null,
        };
        Listing {
            board: "b1".into(),
            link: "L".into(),
            edit_right: EditRight::Editable,
            rectangle: None,
            matched,
            from: 0,
            objects: vec![ListedObject {
                link: "L/a1".into(),
                object,
                drawn: None,
                file: None,
                card: None,
                tags: Vec::new(),
                reactions: std::collections::BTreeMap::new(),
                style: TextStyle::default(),
                detail: Vec::new(),
            }],
            next: None,
        }
    }

    #[test]
    fn a_reference_in_the_parent_of_an_add_item_is_expanded() {
        const ID: &str = "dbc79d73-0000-0000-0000-000000000000";
        let holst = McpServer::offline();
        assert_eq!(holst.state.alias(ID), "@1");
        let arguments = json!({
            "board": "A",
            "items": [{ "text": "x", "parent": "@1" }]
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        let expanded = holst.object_arguments(ObjectVerb::Add, arguments).ok();
        let parent = expanded
            .as_ref()
            .and_then(|given| given.get("items"))
            .and_then(|items| items.get(0))
            .and_then(|item| item.get("parent"));
        assert_eq!(parent, Some(&json!(ID)));
    }

    #[rstest]
    #[case::one(1, "1 object · editable · L\n")]
    #[case::two(2, "2 objects · editable · L\n")]
    fn a_listing_head_counts_its_objects(#[case] matched: usize, #[case] head: &str) {
        let text =
            McpServer::offline().object_listing_text(&one_object_listing(matched));
        assert!(text.starts_with(head), "{text}");
    }

    #[rstest]
    #[case::none(None, Output::Text, LIST_DEFAULT)]
    #[case::none_json(None, Output::Json, LIST_DEFAULT_JSON)]
    #[case::all(Some(0), Output::Text, LIST_MAX)]
    #[case::some(Some(20), Output::Json, 20)]
    #[case::too_many(Some(5_000), Output::Text, LIST_MAX)]
    fn a_listing_has_a_limit(
        #[case] given: Option<u32>,
        #[case] output: Output,
        #[case] shown: u32,
    ) {
        assert_eq!(listed_limit(given, output), shown);
    }

    #[test]
    fn a_downloaded_file_reads_its_size_in_units() {
        let (holst, _) = McpServer::scripted(Vec::<CannedResponse>::new());
        let downloaded = Downloaded {
            board: "b1".to_owned(),
            objects: vec![DownloadedObject {
                object: "a1".to_owned(),
                link: "L".to_owned(),
                delivery: Delivery::Written {
                    file: "/d/a.png".into(),
                    bytes: 1_234_567,
                },
            }],
            not_saved: Vec::new(),
        };
        let text = holst.downloaded_text(&downloaded);
        assert!(text.contains(" saved 1.1 MiB to /d/a.png\n"), "{text}");
    }
}
