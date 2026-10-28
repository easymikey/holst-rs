//! The `object` and `object_write` tools: the objects on a board.

use std::path::Path;

use holst_core::{
    command::{
        board::picture::Delivery,
        object::{
            AddArgs,
            AlignArgs,
            CopyArgs,
            EditArgs,
            GetArgs,
            GroupArgs,
            LockArgs,
            LsArgs,
            ObjectVerb,
            RemoveArgs,
            UngroupArgs,
            UnlockArgs,
            add,
            align,
            change::{self, Removed},
            copy,
            get::{self, Downloaded},
            group,
            list::{self, ListedObject, Listing},
            lock,
        },
        table::{CommandArgs, CommandName, parse},
    },
    domain::{
        change::receipt::ChangeReport,
        text::{one_line, printable},
    },
    effect::{board_write::WriteMode, settings::CacheSettings},
    remote::client::Client,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::HandedClient,
    memory::alias::expand_json,
    reply::{change::written, compact::Output},
    server::McpServer,
    tools::{
        files::{LocalFile, checked_download},
        said,
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
const REFERENCES: [&str; 7] = [
    "board",
    "workspace",
    "frame",
    "object",
    "from",
    "to",
    "link_to",
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
            .try_for_each(|path| LocalFile::check(Path::new(path), &self.secrets))
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
            ObjectVerb::Add => {
                let dry = |args: &AddArgs| args.dry_run;
                self.change_objects(arguments, dry, add::add).await
            }
            ObjectVerb::Edit => {
                let dry = |args: &EditArgs| args.dry_run;
                self.change_objects(arguments, dry, change::edit).await
            }
            ObjectVerb::Cp => {
                let dry = |args: &CopyArgs| args.dry_run;
                self.change_objects(arguments, dry, copy::cp).await
            }
            ObjectVerb::Group => {
                let dry = |args: &GroupArgs| args.dry_run;
                self.change_objects(arguments, dry, group::group).await
            }
            ObjectVerb::Ungroup => {
                let dry = |args: &UngroupArgs| args.dry_run;
                self.change_objects(arguments, dry, group::ungroup).await
            }
            ObjectVerb::Lock => {
                let dry = |args: &LockArgs| args.dry_run;
                self.change_objects(arguments, dry, lock::lock).await
            }
            ObjectVerb::Unlock => {
                let dry = |args: &UnlockArgs| args.dry_run;
                self.change_objects(arguments, dry, lock::unlock).await
            }
            ObjectVerb::Align => {
                let dry = |args: &AlignArgs| args.dry_run;
                self.change_objects(arguments, dry, align::align).await
            }
            ObjectVerb::Rm => self.remove_objects(arguments).await,
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

    /// Reads the arguments of a change of objects and runs it: a dry run
    /// reads only, a write drops what the server remembers.
    async fn change_objects<A: CommandArgs>(
        &self,
        arguments: Map<String, Value>,
        dry_run: impl Fn(&A) -> WriteMode,
        change: impl AsyncFnOnce(
            &Client,
            &CacheSettings,
            &A,
        ) -> holst_core::Result<ChangeReport>,
    ) -> CallToolResult {
        self.parsed(arguments, async |args: A| {
            let call = async |client: HandedClient| -> Result<String> {
                let report = change(&client, &self.cache, &args).await?;
                Ok(self.written_text(&report))
            };
            self.by_mode(dry_run(&args), call).await
        })
        .await
    }

    /// Several objects, or a frame with content, stay on the board without
    /// `confirm`.
    async fn remove_objects(&self, arguments: Map<String, Value>) -> CallToolResult {
        self.parsed(arguments, async |args: RemoveArgs| {
            let call = async |client: HandedClient| -> Result<String> {
                let Removed {
                    report, refusal, ..
                } = change::rm(&client, &self.cache, &args).await?;
                said(refusal.as_ref(), || self.written_text(&report))
            };
            self.by_mode(args.dry_run, call).await
        })
        .await
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

    fn downloaded_text(&self, downloaded: &Downloaded) -> String {
        let saved =
            downloaded
                .objects
                .iter()
                .filter_map(|object| match &object.delivery {
                    Delivery::Written { file, bytes } => Some(format!(
                        "{} saved {bytes} bytes to {}\n",
                        self.state.alias(&object.object),
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
            "{} objects · {} · {}\n",
            listing.matched,
            listing.edit_right.as_str(),
            listing.link
        );
        let lines = listing
            .objects
            .iter()
            .map(|listed| self.object_line(listed) + "\n");
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

    fn object_line(&self, listed: &ListedObject) -> String {
        let object = &listed.object;
        let alias = |id: &str| self.state.alias(id);
        let corner = format!("({:.0}, {:.0})", object.x, object.y);
        let size = object
            .width
            .zip(object.height)
            .map(|(width, height)| format!(" {width:.0}×{height:.0}"))
            .unwrap_or_default();
        let inside = object
            .parent
            .as_deref()
            .map(|parent| format!(" in {}", alias(parent)))
            .unwrap_or_default();
        let color = object
            .color
            .as_deref()
            .map(|color| format!("  {color}"))
            .unwrap_or_default();
        let look = object
            .look
            .as_ref()
            .map(|look| format!("  {}", look.words()))
            .unwrap_or_default();
        let style = listed.style.line();
        let ends = object
            .ends
            .as_ref()
            .map(|ends| {
                let end = |end: &Option<String>| {
                    end.as_deref().map_or_else(|| "·".to_owned(), alias)
                };
                format!("  {} → {}", end(&ends.start), end(&ends.end))
            })
            .unwrap_or_default();
        let tags = if listed.tags.is_empty() {
            String::new()
        } else {
            format!("  #{}", listed.tags.join(" #"))
        };
        let padlock = if object.locked { "  locked" } else { "" };
        let turn = if object.rotation == 0.0 {
            String::new()
        } else {
            format!("  {}°", (object.rotation * 100.0).round() / 100.0)
        };
        let linked = object
            .link_to
            .as_deref()
            .map_or_else(String::new, |link| format!("  link-to={link}"));
        let text = shorten(&one_line(&object.text));
        let quoted = if text.is_empty() {
            String::new()
        } else {
            format!("  \"{}\"", printable(&text))
        };
        format!(
            "{} {:<7} {corner}{size}{inside}{color}{look}{style}{ends}{padlock}{turn}{linked}{tags}{quoted}",
            alias(&object.id),
            object.kind.short_name(),
        )
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

fn shorten(text: &str) -> String {
    text.char_indices().nth(TEXT_MAX_CHARS).map_or_else(
        || text.to_owned(),
        |(cut, _)| format!("{}…", text.get(..cut).unwrap_or(text)),
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{
        LIST_DEFAULT,
        LIST_DEFAULT_JSON,
        LIST_MAX,
        TEXT_MAX_CHARS,
        listed_limit,
        shorten,
    };
    use crate::reply::compact::Output;

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
    fn a_long_text_is_cut_with_an_ellipsis() {
        let long = "x".repeat(TEXT_MAX_CHARS + 5);
        assert_eq!(shorten(&long), format!("{}…", "x".repeat(TEXT_MAX_CHARS)));
    }
}
