//! The MCP agent eval: 20 tasks, the calls that solve them and their checks.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicU32, Ordering},
};

use holst_core::fake::{FixtureObject, FixtureThread, ThreadAnchor};
use rmcp::{
    ServiceExt as _,
    model::CallToolRequestParams,
    transport::TokioChildProcess,
};
use serde_json::{Map, Value, json};

use crate::{
    cli,
    error::ScenarioError,
    given::given,
    holst::{Holst, fields},
    mcp::text_of,
};

/// How many tasks there are.
pub const TASKS: usize = 20;
static TAKEN: AtomicU32 = AtomicU32::new(0);
const WORKSPACE: &str = "Eval";
const ROADMAP: &str = "Roadmap";
const RETRO: &str = "Retro";
const BUDGET: &str = "Budget 2026";
const SERVER: &str = "holst-eval";
const TOOL_PREFIX: &str = "mcp__holst-eval__";
const OTHER_USER: u32 = 12345;
const THREAD: &str = "Is Q3 final?";
/// The fixture objects of Roadmap, numbered from 1.
const Q3: u32 = 1;
const Q4: u32 = 2;
const ASK_DESIGN: u32 = 3;
const SHIP: u32 = 4;
const OLD_IDEA: u32 = 5;
const START: u32 = 6;
const LOGO: u32 = 7;
const SPEC: u32 = 8;
const FIXTURE_OBJECTS: u32 = 8;
const STAMPS: [&str; 3] = ["created", "updated", "listed"];

/// One call of a tool, as an agent makes it.
#[derive(Debug, Clone)]
pub struct Call {
    pub tool: &'static str,
    pub arguments: Value,
}

/// What a tool answered.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub text: String,
    pub is_error: bool,
}

/// What the fake holds: the objects of every board and the threads of Roadmap.
#[derive(Debug, Clone)]
pub struct Snapshot {
    boards: BTreeMap<String, Vec<Value>>,
    names: Vec<String>,
    threads: Vec<FixtureThread>,
    ids: Vec<String>,
    /// Each board as `board export --no-compress` saves it: a Yjs update.
    exports: BTreeMap<String, Vec<u8>>,
}

/// The fake before a run and after it.
#[derive(Debug)]
pub struct Seen<'a> {
    before: &'a Snapshot,
    after: &'a Snapshot,
}

/// Why a check refuses a run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Mismatch(String);

type Check = fn(&Seen<'_>) -> Result<(), Mismatch>;

/// One task: what the user says, the fewest calls that do it, and the check.
#[derive(Debug)]
pub struct Task {
    pub number: usize,
    pub covers: &'static str,
    pub prompt: &'static str,
    pub ideal: usize,
    pub calls: Vec<Call>,
    check: Check,
}

/// How the check judged one run.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub correct: bool,
    pub reason: String,
}

/// What the oracle found for one task.
#[derive(Debug)]
pub struct OracleLine {
    pub number: usize,
    pub calls: usize,
    pub ideal: usize,
    pub right_answer: Result<(), Mismatch>,
    pub untouched_fixture: Result<(), Mismatch>,
}

impl OracleLine {
    /// Whether the check accepts the ideal calls and refuses the untouched
    /// fixture.
    #[must_use]
    pub fn proven(&self) -> bool {
        self.right_answer.is_ok() && self.untouched_fixture.is_err()
    }
}

fn call(tool: &'static str, arguments: Value) -> Call {
    Call { tool, arguments }
}

fn write(arguments: Value) -> Call {
    call("object_write", arguments)
}

/// Builds the fixture every task starts from.
///
/// # Errors
/// As [`ScenarioError`].
pub async fn fixture() -> Result<Holst, ScenarioError> {
    let frame = |label: &str, x: i32| {
        FixtureObject::Frame(label.into()).at(x, 0).sized(800, 600)
    };
    let objects = vec![
        frame("Q3", 0),
        frame("Q4", 1000),
        FixtureObject::Sticker("Ask design".into())
            .at(100, 100)
            .inside(Q3),
        FixtureObject::Sticker("Ship".into())
            .at(1100, 100)
            .inside(Q4),
        FixtureObject::Sticker("Old idea".into()).at(2000, 100),
        FixtureObject::Shape("Start".into()).at(100, 700),
        FixtureObject::Image("logo.png".into())
            .sized(400, 300)
            .at(600, 700),
        FixtureObject::File("Spec.pdf".into(), 70)
            .set("pagesInfo", json!({"count": 5, "valid": true}))
            .at(1100, 700),
    ];
    let thread = FixtureThread::new("q3final", OTHER_USER, THREAD)
        .reply_by_me("typo")
        .reply(OTHER_USER, "agreed");
    given()
        .saved_login()
        .workspace(WORKSPACE)
        .board_with_objects(ROADMAP, WORKSPACE, objects)
        .board(RETRO, WORKSPACE, None)
        .board_with_objects(
            BUDGET,
            WORKSPACE,
            vec![FixtureObject::Sticker("budget review".into())],
        )
        .thread(ROADMAP, thread)
        .page_count("Spec.pdf", 5)
        .start()
        .await
}

/// Runs `calls` one after the other in one session of `holst-mcp`.
///
/// # Errors
/// As [`ScenarioError`].
pub async fn run_calls(
    holst: &Holst,
    calls: &[Call],
) -> Result<Vec<Outcome>, ScenarioError> {
    let _running = cli::RUNNING.acquire().await;
    let client = ()
        .serve(TokioChildProcess::new(tokio::process::Command::from(
            holst.command("holst-mcp")?,
        ))?)
        .await?;
    let mut outcomes = Vec::new();
    // imperative: each call waits for the reply of the one before
    for next in calls {
        let reply = client
            .call_tool(
                CallToolRequestParams::new(next.tool)
                    .with_arguments(fields(next.arguments.clone())),
            )
            .await?;
        outcomes.push(Outcome {
            text: text_of(&reply),
            is_error: reply.is_error == Some(true),
        });
    }
    client.cancel().await?;
    Ok(outcomes)
}

impl Snapshot {
    /// Reads what the fake holds now; the texts of the objects come through
    /// the `object` tool.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn take(holst: &Holst) -> Result<Self, ScenarioError> {
        let names: Vec<String> = holst
            .fake
            .tree_of(WORKSPACE)
            .into_iter()
            .map(|node| node.name)
            .collect();
        let reads: Vec<Call> = names
            .iter()
            .map(|name| {
                call(
                    "object",
                    json!({"action": "ls", "board": name, "output": "json", "limit": 0}),
                )
            })
            .collect();
        let taken = TAKEN.fetch_add(1, Ordering::Relaxed);
        let exports: Vec<Call> = names
            .iter()
            .zip(0_u32..)
            .map(|(name, at)| {
                let file = holst.work_place(&format!("export-{taken}-{at}"));
                call(
                    "board_write",
                    json!({"action": "export", "board": name, "file": file, "no_compress": true}),
                )
            })
            .collect();
        let replies =
            run_calls(holst, &reads.into_iter().chain(exports).collect::<Vec<_>>())
                .await?;
        let saved: BTreeMap<String, Vec<u8>> = names
            .iter()
            .zip(0_u32..)
            .map(|(name, at)| {
                let file = holst.work_place(&format!("export-{taken}-{at}"));
                (name.clone(), std::fs::read(file).unwrap_or_default())
            })
            .collect();
        let boards = names
            .iter()
            .zip(&replies)
            .map(|(name, reply)| {
                let listing = listed_in(&reply.text);
                let objects = holst
                    .fake
                    .objects_of(name)
                    .into_iter()
                    .map(|object| with_listing(object, &listing))
                    .collect();
                (name.clone(), objects)
            })
            .collect();
        let ids = (1..=FIXTURE_OBJECTS)
            .filter_map(|number| holst.fake.object_id(ROADMAP, number))
            .collect();
        Ok(Self {
            boards,
            names,
            threads: holst.fake.threads_of(ROADMAP),
            ids,
            exports: saved,
        })
    }
}

/// The objects of an `object ls` reply in JSON.
fn listed_in(reply: &str) -> Vec<Value> {
    let parsed: Value = serde_json::from_str(reply).unwrap_or_default();
    match &parsed {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map
            .values()
            .flat_map(|value| value.as_array().cloned().unwrap_or_default())
            .collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            Vec::new()
        }
    }
}

/// The object with what `object ls` said of it under `listed`.
fn with_listing(object: Value, listing: &[Value]) -> Value {
    let id = object.get("id").and_then(Value::as_str).unwrap_or_default();
    let listed = listing.iter().find(|item| {
        item.get("id")
            .and_then(Value::as_str)
            .is_some_and(|shown| id.starts_with(shown) || shown.starts_with(id))
    });
    Value::Object(
        fields(object)
            .into_iter()
            .chain(listed.map(|item| ("listed".to_owned(), item.clone())))
            .collect(),
    )
}

fn without_stamps(object: &Value) -> Value {
    Value::Object(
        fields(object.clone())
            .into_iter()
            .filter(|(key, _)| !STAMPS.contains(&key.as_str()))
            .collect(),
    )
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// The checkbox states of a board export in reading order: `x` ticked, `y` not.
fn checkbox_states(export: &[u8]) -> String {
    let item = b"checkbox-list-item";
    (0..export.len())
        .filter(|at| export.get(*at..*at + item.len()) == Some(item))
        .filter_map(|at| {
            let id = find(export, b"w$", at)?;
            export.get(id + 2 + 36).copied()
        })
        .map(char::from)
        .collect()
}

fn ensure(holds: bool, what: impl Into<String>) -> Result<(), Mismatch> {
    holds.then_some(()).ok_or_else(|| Mismatch(what.into()))
}

fn text(object: &Value) -> &str {
    at(object, "/listed/text")
        .or_else(|| at(object, "/text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn at<'v>(object: &'v Value, pointer: &str) -> Option<&'v Value> {
    object.pointer(pointer)
}

fn number(object: &Value, pointer: &str) -> f64 {
    at(object, pointer)
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN)
}

fn named(object: &Value, pointer: &str, wanted: &str) -> bool {
    at(object, pointer).and_then(Value::as_str) == Some(wanted)
}

/// What a check lets a run change.
#[derive(Debug, Default, Clone, Copy)]
struct Touched<'a> {
    /// Boards that may gain objects.
    boards: &'a [&'a str],
    objects: &'a [u32],
    names: bool,
    threads: bool,
}

impl Seen<'_> {
    fn id(&self, number: u32) -> &str {
        usize::try_from(number)
            .ok()
            .and_then(|at| self.before.ids.get(at.wrapping_sub(1)))
            .map_or("", String::as_str)
    }

    fn roadmap(snapshot: &Snapshot) -> &[Value] {
        snapshot.boards.get(ROADMAP).map_or(&[], Vec::as_slice)
    }

    /// The fixture object `number` as it is now, if it is still there.
    fn object(&self, number: u32) -> Option<&Value> {
        Self::roadmap(self.after)
            .iter()
            .find(|object| named(object, "/id", self.id(number)))
    }

    fn fixture(&self, number: u32) -> Result<&Value, Mismatch> {
        self.object(number)
            .ok_or_else(|| Mismatch(format!("fixture object {number} is gone")))
    }

    fn export(&self, board: &str) -> &[u8] {
        self.after.exports.get(board).map_or(&[], Vec::as_slice)
    }

    /// The objects on `board` that were not there before.
    fn added(&self, board: &str) -> Vec<&Value> {
        let known: Vec<&str> = self
            .before
            .boards
            .get(board)
            .into_iter()
            .flatten()
            .filter_map(|object| object.get("id").and_then(Value::as_str))
            .collect();
        self.after
            .boards
            .get(board)
            .into_iter()
            .flatten()
            .filter(|object| {
                object
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !known.contains(&id))
            })
            .collect()
    }

    /// The one object added on `board`, of this document type.
    fn one_added(&self, board: &str, kind: &str) -> Result<&Value, Mismatch> {
        let added = self.added(board);
        let wanted: Vec<&&Value> = added
            .iter()
            .filter(|object| named(object, "/type", kind))
            .collect();
        match wanted.as_slice() {
            [only] => Ok(only),
            _ => Err(Mismatch(format!(
                "expected one new {kind} on {board}, found {}",
                Value::Array(added.iter().map(|object| (*object).clone()).collect())
            ))),
        }
    }

    /// Everything the run did not name is as it was.
    fn rest_unchanged(&self, touched: Touched<'_>) -> Result<(), Mismatch> {
        let skipped: Vec<&str> = touched
            .objects
            .iter()
            .map(|number| self.id(*number))
            .collect();
        self.before.boards.iter().try_for_each(|(name, objects)| {
            let free = touched.boards.contains(&name.as_str());
            let renamed = touched.names && !self.after.boards.contains_key(name);
            let now = self.after.boards.get(name).map_or(&[][..], Vec::as_slice);
            let kept = objects
                .iter()
                .filter(|object| {
                    let id = object.get("id").and_then(Value::as_str).unwrap_or("");
                    !(free && skipped.contains(&id))
                })
                .all(|object| {
                    now.iter().any(|candidate| {
                        without_stamps(candidate) == without_stamps(object)
                    })
                });
            ensure(
                renamed || (kept && (free || now.len() == objects.len())),
                format!("the board {name} changed"),
            )
        })?;
        ensure(
            touched.names || self.before.names == self.after.names,
            "the boards were renamed or moved",
        )?;
        ensure(
            touched.threads || self.before.threads == self.after.threads,
            "the comment threads changed",
        )
    }
}

impl Verdict {
    fn into_result(self) -> Result<(), Mismatch> {
        self.correct.then_some(()).ok_or(Mismatch(self.reason))
    }
}

impl Task {
    fn new(
        number: usize,
        (covers, prompt): (&'static str, &'static str),
        calls: Vec<Call>,
        check: Check,
    ) -> Self {
        Self {
            number,
            covers,
            prompt,
            ideal: calls.len(),
            calls,
            check,
        }
    }

    /// Judges the run that took `before` to `after`.
    #[must_use]
    pub fn judge(&self, before: &Snapshot, after: &Snapshot) -> Verdict {
        let seen = Seen { before, after };
        let outcome = (self.check)(&seen);
        Verdict {
            correct: outcome.is_ok(),
            reason: outcome.err().map(|why| why.to_string()).unwrap_or_default(),
        }
    }
}

const ROADMAP_ONLY: Touched<'static> = Touched {
    boards: &[ROADMAP],
    objects: &[],
    names: false,
    threads: false,
};

const RETRO_ONLY: Touched<'static> = Touched {
    boards: &[RETRO],
    objects: &[],
    names: false,
    threads: false,
};

fn on_roadmap_touching(objects: &[u32]) -> Touched<'_> {
    Touched {
        objects,
        ..ROADMAP_ONLY
    }
}

fn added_in_frame(seen: &Seen<'_>) -> Result<(), Mismatch> {
    let sticker = seen.one_added(ROADMAP, "sticker")?;
    ensure(
        text(sticker) == "Call Ann",
        format!("sticker text {sticker}"),
    )?;
    ensure(
        named(sticker, "/parentId", seen.id(Q3)),
        format!("not inside Q3: {sticker}"),
    )?;
    seen.rest_unchanged(ROADMAP_ONLY)
}

/// The 20 tasks, in order, with the calls that solve them on the fixture of
/// `holst`.
#[must_use]
pub fn tasks(holst: &Holst) -> Vec<Task> {
    let ctx = Ctx {
        ids: (1..=FIXTURE_OBJECTS)
            .filter_map(|number| holst.fake.object_id(ROADMAP, number))
            .collect(),
    };
    [
        task_1, task_2, task_3, task_4, task_5, task_6, task_7, task_8, task_9,
        task_10, task_11, task_12, task_13, task_14, task_15, task_16, task_17,
        task_18, task_19, task_20,
    ]
    .iter()
    .map(|task| task(&ctx))
    .collect()
}

/// What the calls of a task are built from.
struct Ctx {
    ids: Vec<String>,
}

impl Ctx {
    fn id(&self, number: u32) -> String {
        usize::try_from(number)
            .ok()
            .and_then(|at| self.ids.get(at.wrapping_sub(1)))
            .cloned()
            .unwrap_or_default()
    }
}

fn list() -> Call {
    call("object", json!({"action": "ls", "board": ROADMAP}))
}

fn threads() -> Call {
    call("comment", json!({"action": "ls", "board": ROADMAP}))
}

fn task_1(_ctx: &Ctx) -> Task {
    Task::new(
        1,
        (
            "add into frame",
            "On Roadmap, put a sticker \"Call Ann\" into the frame Q3.",
        ),
        vec![write(json!({
            "action": "add", "kind": "sticker", "board": ROADMAP,
            "text": "Call Ann", "frame": "Q3"
        }))],
        added_in_frame,
    )
}

fn task_2(ctx: &Ctx) -> Task {
    Task::new(
        2,
        (
            "arrow",
            "Connect the sticker \"Ask design\" to \"Ship\" with an arrow.",
        ),
        vec![
            list(),
            write(json!({
                "action": "add", "kind": "arrow", "board": ROADMAP,
                "from": ctx.id(ASK_DESIGN), "to": ctx.id(SHIP)
            })),
        ],
        |seen| {
            let arrow = seen.one_added(ROADMAP, "arrow")?;
            ensure(
                named(arrow, "/start/objectId", seen.id(ASK_DESIGN))
                    && named(arrow, "/end/objectId", seen.id(SHIP)),
                format!("the arrow does not join them: {arrow}"),
            )?;
            seen.rest_unchanged(ROADMAP_ONLY)
        },
    )
}

fn task_3(ctx: &Ctx) -> Task {
    Task::new(
        3,
        (
            "move into frame",
            "Move the sticker \"Old idea\" into the frame Q4.",
        ),
        vec![
            list(),
            write(json!({
                "action": "edit", "board": ROADMAP,
                "object": [ctx.id(OLD_IDEA)], "frame": "Q4"
            })),
        ],
        |seen| {
            let idea = seen.fixture(OLD_IDEA)?;
            let frame = seen.fixture(Q4)?;
            ensure(
                named(idea, "/parentId", seen.id(Q4)),
                format!("not inside Q4: {idea}"),
            )?;
            let inside = number(idea, "/position/x") >= number(frame, "/position/x")
                && number(idea, "/position/y") >= number(frame, "/position/y")
                && number(idea, "/position/x") + number(idea, "/width")
                    <= number(frame, "/position/x") + number(frame, "/width")
                && number(idea, "/position/y") + number(idea, "/height")
                    <= number(frame, "/position/y") + number(frame, "/height");
            ensure(inside, format!("outside the box of Q4: {idea}"))?;
            seen.rest_unchanged(on_roadmap_touching(&[OLD_IDEA]))
        },
    )
}

fn task_4(ctx: &Ctx) -> Task {
    Task::new(
        4,
        (
            "risky remove",
            "Delete the frame Q4 with everything in it; I agree to that.",
        ),
        vec![
            list(),
            write(json!({
                "action": "rm", "board": ROADMAP,
                "object": [ctx.id(Q4)], "confirm": true
            })),
        ],
        |seen| {
            ensure(
                seen.object(Q4).is_none() && seen.object(SHIP).is_none(),
                "Q4 or Ship is still there",
            )?;
            seen.rest_unchanged(on_roadmap_touching(&[Q4, SHIP]))
        },
    )
}

fn task_5(_ctx: &Ctx) -> Task {
    Task::new(
        5,
        (
            "search, board",
            "Which board mentions \"budget review\"? Rename it to \"Budget 2026 (old)\".",
        ),
        vec![
            call(
                "search",
                json!({"query": "budget review", "mode": "board_text"}),
            ),
            call(
                "board_write",
                json!({"action": "rename", "board": BUDGET, "name": "Budget 2026 (old)"}),
            ),
        ],
        |seen| {
            ensure(
                seen.after
                    .names
                    .iter()
                    .any(|name| name == "Budget 2026 (old)")
                    && !seen.after.names.iter().any(|name| name == BUDGET),
                format!("the boards are {:?}", seen.after.names),
            )?;
            seen.rest_unchanged(Touched {
                names: true,
                ..Touched::default()
            })
        },
    )
}

fn task_6(_ctx: &Ctx) -> Task {
    Task::new(
        6,
        (
            "W2 rich text",
            "On Retro, add a sticker with a checklist Plan, Build, Ship, and tick Plan.",
        ),
        vec![write(json!({
            "action": "add", "kind": "sticker", "board": RETRO,
            "text": "- [x] Plan\n- [ ] Build\n- [ ] Ship",
            "text_format": "markdown"
        }))],
        |seen| {
            let sticker = seen.one_added(RETRO, "sticker")?;
            ensure(
                text(sticker) == "Plan\nBuild\nShip"
                    && checkbox_states(seen.export(RETRO)) == "xyy",
                format!(
                    "the checklist reads {sticker}, boxes {}",
                    checkbox_states(seen.export(RETRO))
                ),
            )?;
            seen.rest_unchanged(RETRO_ONLY)
        },
    )
}

fn task_7(ctx: &Ctx) -> Task {
    Task::new(
        7,
        (
            "W2 lock",
            "Lock \"Ask design\" and \"Ship\" together so nobody moves them.",
        ),
        vec![
            list(),
            write(json!({
                "action": "lock", "board": ROADMAP,
                "object": [ctx.id(ASK_DESIGN), ctx.id(SHIP)]
            })),
        ],
        |seen| {
            locked_group_of_both(seen)?;
            seen.rest_unchanged(on_roadmap_touching(&[ASK_DESIGN, SHIP]))
        },
    )
}

/// One new locked group holds both members, directly or through new groups.
fn locked_group_of_both(seen: &Seen<'_>) -> Result<(), Mismatch> {
    let added = seen.added(ROADMAP);
    let id_of = |object: &Value| {
        object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let all_groups = added.iter().all(|object| named(object, "/type", "group"));
    let locked: Vec<&&Value> = added
        .iter()
        .filter(|object| at(object, "/locked") == Some(&Value::Bool(true)))
        .collect();
    let top_id = match locked.as_slice() {
        [top] => Ok(id_of(top)),
        _ => Err(Mismatch(format!("not one new locked group: {added:?}"))),
    }?;
    let parent = |object: &Value| {
        object
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let reaches = |member: u32| {
        std::iter::successors(seen.object(member).and_then(parent), |id| {
            added
                .iter()
                .find(|object| id_of(object) == *id)
                .and_then(|object| parent(object))
        })
        .take(added.len() + 1)
        .any(|id| id == top_id)
    };
    ensure(
        all_groups && reaches(ASK_DESIGN) && reaches(SHIP),
        format!("not one locked group of both: {top_id}"),
    )
}

fn task_8(ctx: &Ctx) -> Task {
    Task::new(
        8,
        (
            "W2 shape style",
            "Make the shape \"Start\" an ellipse with a dashed red outline.",
        ),
        vec![
            list(),
            write(json!({
                "action": "edit", "board": ROADMAP, "object": [ctx.id(START)],
                "shape": "ellipse", "stroke_style": "dashed", "stroke_color": "red6"
            })),
        ],
        |seen| {
            let shape = seen.fixture(START)?;
            let red = at(shape, "/strokeColor/color")
                .and_then(Value::as_str)
                .is_some_and(|color| color.starts_with("red"));
            ensure(
                named(shape, "/shapeType", "ellipse")
                    && named(shape, "/strokeStyle", "dashed")
                    && red,
                format!("the shape is {shape}"),
            )?;
            seen.rest_unchanged(on_roadmap_touching(&[START]))
        },
    )
}

fn task_9(ctx: &Ctx) -> Task {
    Task::new(
        9,
        (
            "W2 align",
            "Line up \"Ask design\", \"Ship\" and \"Old idea\" along their left edges.",
        ),
        vec![
            list(),
            write(json!({
                "action": "align", "board": ROADMAP,
                "object": [ctx.id(ASK_DESIGN), ctx.id(SHIP), ctx.id(OLD_IDEA)], "side": "left"
            })),
        ],
        |seen| {
            let lefts: Vec<f64> = [ASK_DESIGN, SHIP, OLD_IDEA]
                .iter()
                .map(|member| {
                    seen.object(*member)
                        .map_or(f64::NAN, |object| number(object, "/position/x"))
                })
                .collect();
            ensure(
                lefts
                    .windows(2)
                    .all(|pair| matches!(pair, [a, b] if (a - b).abs() < 0.5)),
                format!("left edges {lefts:?}"),
            )?;
            seen.rest_unchanged(on_roadmap_touching(&[ASK_DESIGN, SHIP, OLD_IDEA]))
        },
    )
}

fn task_10(ctx: &Ctx) -> Task {
    Task::new(
        10,
        (
            "W2 arrow look",
            "Add a curved arrow from \"Ask design\" to \"Ship\" labelled \"yes\".",
        ),
        vec![
            list(),
            write(json!({
                "action": "add", "kind": "arrow", "board": ROADMAP,
                "from": ctx.id(ASK_DESIGN), "to": ctx.id(SHIP), "line": "curved",
                "label": ["yes"]
            })),
        ],
        |seen| {
            let arrow = seen.one_added(ROADMAP, "arrow")?;
            ensure(
                named(arrow, "/arrowType", "curved")
                    && named(arrow, "/start/objectId", seen.id(ASK_DESIGN))
                    && named(arrow, "/end/objectId", seen.id(SHIP))
                    && arrow.to_string().contains("yes"),
                format!("the arrow is {arrow}"),
            )?;
            seen.rest_unchanged(ROADMAP_ONLY)
        },
    )
}

fn task_11(_ctx: &Ctx) -> Task {
    Task::new(
        11,
        (
            "W3 kanban",
            "On Retro, make a kanban with the columns To do, Doing, Done.",
        ),
        vec![write(json!({
            "action": "add", "kind": "kanban", "board": RETRO,
            "column": ["To do", "Doing", "Done"]
        }))],
        |seen| {
            let kanban = seen.one_added(RETRO, "kanban")?;
            let export = seen.export(RETRO);
            let todo = find(export, b"To do", 0);
            let doing = todo.and_then(|at| find(export, b"Doing", at));
            let done = doing.and_then(|at| find(export, b"Done", at));
            ensure(
                done.is_some(),
                format!("the columns are not in order: {kanban}"),
            )?;
            seen.rest_unchanged(RETRO_ONLY)
        },
    )
}

fn task_12(_ctx: &Ctx) -> Task {
    Task::new(
        12,
        (
            "W3 table",
            "On Retro, add a table with 3 columns and 4 rows; the first row reads Name, Owner, Due.",
        ),
        vec![
            write(json!({
                "action": "add", "kind": "table", "board": RETRO,
                "rows": 4, "columns": 3
            })),
            write(json!({
                "action": "edit", "board": RETRO,
                "items": [
                    {"object": "@1", "cell": "1,1", "text": "Name"},
                    {"object": "@1", "cell": "1,2", "text": "Owner"},
                    {"object": "@1", "cell": "1,3", "text": "Due"}
                ]
            })),
        ],
        |seen| {
            let table = seen.one_added(RETRO, "table")?;
            let table_id = table.get("id").and_then(Value::as_str).unwrap_or("-");
            let mut first_row: Vec<(String, String)> = seen
                .added(RETRO)
                .into_iter()
                .filter(|cell| {
                    named(cell, "/type", "table-cell")
                        && named(cell, "/parentId", table_id)
                        && at(cell, "/rowId") == Some(&json!(1))
                })
                .map(|cell| {
                    let column = at(cell, "/columnId").map(Value::to_string);
                    (column.unwrap_or_default(), text(cell).to_owned())
                })
                .collect();
            first_row.sort();
            let texts: Vec<&str> =
                first_row.iter().map(|(_, text)| text.as_str()).collect();
            ensure(
                texts == ["Name", "Owner", "Due"]
                    && at(table, "/listed/detail/rows") == Some(&json!(4))
                    && at(table, "/listed/detail/columns") == Some(&json!(3)),
                format!("the table is {table}, row 1 {texts:?}"),
            )?;
            seen.rest_unchanged(RETRO_ONLY)
        },
    )
}

fn task_13(_ctx: &Ctx) -> Task {
    Task::new(
        13,
        (
            "W3 task card",
            "On Retro, add a task \"Release 0.3\" due 2026-11-15, status in progress.",
        ),
        vec![write(json!({
            "action": "add", "kind": "task", "board": RETRO,
            "text": "Release 0.3", "due": "2026-11-15", "status": "in-progress"
        }))],
        |seen| {
            let task = seen.one_added(RETRO, "task-card")?;
            ensure(
                text(task) == "Release 0.3"
                    && named(task, "/schedule/end/date", "2026-11-15")
                    && named(task, "/statusId", "in-progress"),
                format!("the task is {task}"),
            )?;
            seen.rest_unchanged(RETRO_ONLY)
        },
    )
}

fn task_14(_ctx: &Ctx) -> Task {
    Task::new(
        14,
        (
            "W3 mind map",
            "On Retro, start a mind map \"Plan\" with two branches \"Build\" and \"Test\".",
        ),
        vec![
            write(json!({
                "action": "add", "kind": "mind-map", "board": RETRO, "text": "Plan"
            })),
            write(json!({
                "action": "add", "kind": "mind-map", "board": RETRO,
                "items": [
                    {"text": "Build", "parent": "@1"},
                    {"text": "Test", "parent": "@1"}
                ]
            })),
        ],
        |seen| {
            let nodes = seen.added(RETRO);
            let root = nodes.iter().find(|node| text(node) == "Plan");
            let root_id = root
                .and_then(|node| node.get("id").and_then(Value::as_str))
                .unwrap_or("-");
            let branches = ["Build", "Test"].iter().all(|branch| {
                nodes.iter().any(|node| {
                    text(node) == *branch && named(node, "/parentId", root_id)
                })
            });
            ensure(
                nodes.len() == 3 && branches,
                format!(
                    "the nodes are {}",
                    nodes.iter().map(|node| (*node).clone()).collect::<Value>()
                ),
            )?;
            seen.rest_unchanged(RETRO_ONLY)
        },
    )
}

fn task_15(ctx: &Ctx) -> Task {
    Task::new(
        15,
        (
            "W3 reaction",
            "React with a thumbs-up to the sticker \"Ship\".",
        ),
        vec![
            list(),
            write(json!({
                "action": "edit", "board": ROADMAP, "object": [ctx.id(SHIP)],
                "reaction": ["👍"]
            })),
        ],
        |seen| {
            let ship = seen.fixture(SHIP)?;
            ensure(
                at(ship, "/listed/reactions/👍") == Some(&json!(1)),
                format!("no reaction on Ship: {ship}"),
            )?;
            seen.rest_unchanged(on_roadmap_touching(&[SHIP]))
        },
    )
}

fn task_16(ctx: &Ctx) -> Task {
    Task::new(
        16,
        (
            "W4 image",
            "Crop the logo image to a circle and call it Logo.png.",
        ),
        vec![
            list(),
            write(json!({
                "action": "edit", "board": ROADMAP, "object": [ctx.id(LOGO)],
                "crop": "circle", "name": "Logo.png"
            })),
        ],
        |seen| {
            let logo = seen.fixture(LOGO)?;
            ensure(
                (number(logo, "/cropTransform/4") - 1.0).abs() < f64::EPSILON
                    && named(logo, "/originalName", "Logo.png"),
                format!("the image is {logo}"),
            )?;
            seen.rest_unchanged(on_roadmap_touching(&[LOGO]))
        },
    )
}

fn task_17(_ctx: &Ctx) -> Task {
    Task::new(
        17,
        (
            "W4 drawing",
            "With the highlighter, underline the sticker \"Ship\".",
        ),
        vec![
            list(),
            write(json!({
                "action": "add", "kind": "drawing", "board": ROADMAP,
                "highlighter": true, "point": ["1100,296", "1292,296"]
            })),
        ],
        |seen| {
            let drawing = seen.one_added(ROADMAP, "drawing")?;
            let ship = seen.fixture(SHIP)?;
            let bottom = number(ship, "/position/y") + number(ship, "/height");
            let along: Vec<f64> = at(drawing, "/path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .split(',')
                .step_by(2)
                .filter_map(|x| x.parse().ok())
                .collect();
            let span = along.iter().copied().fold(f64::MIN, f64::max)
                - along.iter().copied().fold(f64::MAX, f64::min);
            ensure(
                (number(drawing, "/alpha") - 0.5).abs() < f64::EPSILON
                    && (number(drawing, "/position/y") - bottom).abs() <= 20.0
                    && span >= 0.8 * number(ship, "/width"),
                format!("the drawing is {drawing}"),
            )?;
            seen.rest_unchanged(ROADMAP_ONLY)
        },
    )
}

fn task_18(ctx: &Ctx) -> Task {
    Task::new(
        18,
        ("W4 file", "Show Spec.pdf as pages, starting at page 2."),
        vec![
            list(),
            write(json!({
                "action": "edit", "board": ROADMAP, "object": [ctx.id(SPEC)],
                "display": "content", "start_page": 2
            })),
        ],
        |seen| {
            let file = seen.fixture(SPEC)?;
            ensure(
                named(file, "/displayType", "content")
                    && (number(file, "/pinnedPage") - 2.0).abs() < f64::EPSILON,
                format!("the file is {file}"),
            )?;
            seen.rest_unchanged(on_roadmap_touching(&[SPEC]))
        },
    )
}

fn task_19(ctx: &Ctx) -> Task {
    Task::new(
        19,
        (
            "W4 comments",
            "Colour the thread \"Is Q3 final?\" red and pin it on the sticker \"Ship\".",
        ),
        vec![
            list(),
            threads(),
            call(
                "comment",
                json!({
                    "action": "edit", "board": ROADMAP, "thread": "@1",
                    "color": "red6", "object": ctx.id(SHIP)
                }),
            ),
        ],
        |seen| {
            let thread = seen
                .after
                .threads
                .iter()
                .find(|thread| thread.id == "q3final");
            let red = thread
                .and_then(|thread| thread.color.as_deref())
                .is_some_and(|color| color.starts_with("red"));
            let on_ship = thread
                .is_some_and(|thread| thread.anchor == ThreadAnchor::Object(SHIP));
            ensure(red && on_ship, format!("the thread is {thread:?}"))?;
            seen.rest_unchanged(Touched {
                threads: true,
                ..Touched::default()
            })
        },
    )
}

fn task_20(_ctx: &Ctx) -> Task {
    Task::new(
        20,
        (
            "W4 comments",
            "Delete my reply \"typo\" in the thread \"Is Q3 final?\".",
        ),
        vec![
            threads(),
            call(
                "comment",
                json!({
                    "action": "rm", "board": ROADMAP, "thread": "@1",
                    "reply": "1", "confirm": true
                }),
            ),
        ],
        |seen| {
            let messages: Vec<&str> = seen
                .after
                .threads
                .iter()
                .flat_map(|thread| {
                    thread.messages.iter().map(|message| message.text.as_str())
                })
                .collect();
            ensure(
                messages == [THREAD, "agreed"],
                format!("the messages are {messages:?}"),
            )?;
            seen.rest_unchanged(Touched {
                threads: true,
                ..Touched::default()
            })
        },
    )
}

/// The fresh fixture and the `index`th task on it, if there is one.
async fn fresh(index: usize) -> Result<Option<(Holst, Task)>, ScenarioError> {
    let holst = fixture().await?;
    let task = tasks(&holst).into_iter().nth(index);
    Ok(task.map(|task| (holst, task)))
}

/// Runs the ideal calls of the `index`th task on a fresh fixture, then its
/// check, and the check again on the untouched fixture.
///
/// # Errors
/// As [`ScenarioError`].
pub async fn prove(index: usize) -> Result<Option<OracleLine>, ScenarioError> {
    match fresh(index).await? {
        Some((holst, task)) => prove_on(&holst, &task).await.map(Some),
        None => Ok(None),
    }
}

async fn prove_on(holst: &Holst, task: &Task) -> Result<OracleLine, ScenarioError> {
    let before = Snapshot::take(holst).await?;
    let outcomes = run_calls(holst, &task.calls).await?;
    let failed = outcomes
        .iter()
        .zip(&task.calls)
        .find(|(outcome, _)| outcome.is_error)
        .map(|(outcome, sent)| {
            format!("{} {}: {}", sent.tool, sent.arguments, outcome.text)
        });
    let after = Snapshot::take(holst).await?;
    let verdict = task.judge(&before, &after);
    let untouched = task.judge(&before, &before);
    Ok(OracleLine {
        number: task.number,
        calls: task.calls.len(),
        ideal: task.ideal,
        right_answer: failed
            .map_or_else(|| verdict.into_result(), |why| Err(Mismatch(why))),
        untouched_fixture: untouched.into_result(),
    })
}

/// Proves every task, one after the other.
///
/// # Errors
/// As [`ScenarioError`].
pub async fn oracle() -> Result<Vec<OracleLine>, ScenarioError> {
    let mut lines = Vec::new();
    // imperative: one task after the other, each on its own fixture
    for index in 0..TASKS {
        lines.extend(prove(index).await?);
    }
    Ok(lines)
}

/// Where the config of an MCP client for `holst` is written, and its text.
fn client_config(holst: &Holst) -> Result<String, ScenarioError> {
    let command = holst.command("holst-mcp")?;
    let env: Map<String, Value> = command
        .get_envs()
        .filter_map(|(name, value)| {
            Some((
                name.to_string_lossy().into_owned(),
                Value::from(value?.to_string_lossy().into_owned()),
            ))
        })
        .collect();
    Ok(json!({
        "mcpServers": {
            SERVER: {
                "command": command.get_program().to_string_lossy(),
                "env": env,
            }
        }
    })
    .to_string())
}

/// Writes the MCP client config for `holst` and returns its path.
///
/// # Errors
/// As [`ScenarioError`].
pub fn write_config(holst: &Holst) -> Result<PathBuf, ScenarioError> {
    let path = holst.path("mcp-config.json");
    std::fs::write(&path, client_config(holst)?)?;
    Ok(path)
}

/// One tool call of an agent run, read from its stream.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolUse {
    pub tool: String,
    pub arguments: Value,
    pub is_error: bool,
}

impl ToolUse {
    /// Whether the call changes something: a write tool, or a comment action
    /// that is not a read.
    #[must_use]
    pub fn writes(&self) -> bool {
        let action = self.arguments.get("action").and_then(Value::as_str);
        match self.tool.as_str() {
            "object_write" | "board_write" => true,
            "comment" => !matches!(action, Some("ls" | "read") | None),
            _ => false,
        }
    }
}

/// Reads the calls of `holst-eval` tools from the stdout of Claude Code in
/// `--output-format stream-json`: `tool_use` blocks and their `tool_result`.
#[must_use]
pub fn parse_stream(stdout: &str) -> Vec<ToolUse> {
    let blocks: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|event| {
            event
                .pointer("/message/content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let failed: Vec<&str> = blocks
        .iter()
        .filter(|block| named(block, "/type", "tool_result"))
        .filter(|block| at(block, "/is_error") == Some(&Value::Bool(true)))
        .filter_map(|block| block.get("tool_use_id").and_then(Value::as_str))
        .collect();
    blocks
        .iter()
        .filter(|block| named(block, "/type", "tool_use"))
        .filter_map(|block| {
            let tool = block.get("name")?.as_str()?.strip_prefix(TOOL_PREFIX)?;
            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
            Some(ToolUse {
                tool: tool.to_owned(),
                arguments: block.get("input").cloned().unwrap_or_default(),
                is_error: failed.contains(&id),
            })
        })
        .collect()
}

/// Quotes `prompt` for the inside of a double-quoted shell word.
fn quoted(prompt: &str) -> String {
    prompt
        .chars()
        .flat_map(|letter| {
            let escaped = matches!(letter, '\\' | '"' | '$' | '`');
            escaped.then_some('\\').into_iter().chain([letter])
        })
        .collect()
}

/// How the agent command is run for each task and run.
#[derive(Debug, Clone)]
pub struct AgentRun {
    /// A shell command with `{prompt}` (inside double quotes) and `{config}`.
    pub command: String,
    pub runs: usize,
    pub out: PathBuf,
}

/// The result of one agent run.
#[derive(Debug, Clone)]
pub struct Scored {
    pub number: usize,
    pub covers: &'static str,
    pub run: usize,
    pub ideal: usize,
    pub uses: Vec<ToolUse>,
    pub verdict: Verdict,
}

impl Scored {
    /// Whether the first write of the run was answered without a tool error.
    #[must_use]
    pub fn first_write_ok(&self) -> Option<bool> {
        self.uses
            .iter()
            .find(|used| used.writes())
            .map(|used| !used.is_error)
    }

    fn record(&self) -> Value {
        json!({
            "task": self.number,
            "run": self.run,
            "ideal": self.ideal,
            "calls": self.uses.len(),
            "first_write_ok": self.first_write_ok(),
            "correct": self.verdict.correct,
            "reason": self.verdict.reason,
            "tools": self.uses.iter().map(|used| json!({
                "tool": used.tool,
                "arguments": used.arguments,
                "error": used.is_error,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Runs the agent on one task against a fresh fixture and judges the result.
///
/// # Errors
/// As [`ScenarioError`].
pub async fn run_agent(
    index: usize,
    agent: &AgentRun,
    run: usize,
) -> Result<Option<Scored>, ScenarioError> {
    match fresh(index).await? {
        Some((holst, task)) => run_agent_on(&holst, &task, agent, run).await.map(Some),
        None => Ok(None),
    }
}

async fn run_agent_on(
    holst: &Holst,
    task: &Task,
    agent: &AgentRun,
    run: usize,
) -> Result<Scored, ScenarioError> {
    let config = write_config(holst)?;
    let before = Snapshot::take(holst).await?;
    let command = agent
        .command
        .replace("{prompt}", &quoted(task.prompt))
        .replace("{config}", &config.to_string_lossy());
    let output = tokio::process::Command::new("sh")
        .args(["-c", &command])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .await?;
    let after = Snapshot::take(holst).await?;
    Ok(Scored {
        number: task.number,
        covers: task.covers,
        run,
        ideal: task.ideal,
        uses: parse_stream(&String::from_utf8_lossy(&output.stdout)),
        verdict: task.judge(&before, &after),
    })
}

/// Writes `task-NN-run-K.json` for each run and `summary.md` into `out`.
///
/// # Errors
/// [`ScenarioError::Io`] when a file cannot be written.
pub fn write_scores(
    out: &std::path::Path,
    scored: &[Scored],
) -> Result<(), ScenarioError> {
    std::fs::create_dir_all(out)?;
    scored.iter().try_for_each(|run| {
        std::fs::write(
            out.join(format!("task-{:02}-run-{}.json", run.number, run.run)),
            serde_json::to_string_pretty(&run.record())?,
        )
        .map_err(ScenarioError::from)
    })?;
    std::fs::write(out.join("summary.md"), summary(scored))?;
    Ok(())
}

fn summary(scored: &[Scored]) -> String {
    let numbers: BTreeSet<usize> = scored.iter().map(|run| run.number).collect();
    let rows = numbers.iter().map(|number| {
        let own: Vec<&Scored> =
            scored.iter().filter(|run| run.number == *number).collect();
        let (covers, ideal) =
            own.first().map_or(("", 0), |run| (run.covers, run.ideal));
        let correct = own.iter().filter(|run| run.verdict.correct).count();
        let first_ok = own
            .iter()
            .filter(|run| run.first_write_ok() == Some(true))
            .count();
        let calls: Vec<String> =
            own.iter().map(|run| run.uses.len().to_string()).collect();
        format!(
            "| {} | {} | {correct}/{} | {first_ok}/{} | {} | {} |",
            number,
            covers,
            own.len(),
            own.len(),
            ideal,
            calls.join(", ")
        )
    });
    let total = scored.len();
    let correct = scored.iter().filter(|run| run.verdict.correct).count();
    let first = scored
        .iter()
        .filter(|run| run.first_write_ok().is_some())
        .count();
    let first_ok = scored
        .iter()
        .filter(|run| run.first_write_ok() == Some(true))
        .count();
    [
        "# Eval summary".to_owned(),
        String::new(),
        "| # | covers | correct | first write ok | ideal | calls per run |".to_owned(),
        "|---|---|---|---|---|---|".to_owned(),
    ]
    .into_iter()
    .chain(rows)
    .chain([
        String::new(),
        format!("Correct runs: {correct}/{total}. First write ok: {first_ok}/{first}."),
    ])
    .collect::<Vec<_>>()
    .join("\n")
        + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = concat!(
        r#"{"type":"system","subtype":"init"}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"on it"},{"type":"tool_use","id":"t1","name":"mcp__holst-eval__object_write","input":{"action":"add","kind":"sticker"}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"no board"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"mcp__holst-eval__board","input":{"action":"ls"}},{"type":"tool_use","id":"t3","name":"Bash","input":{}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"ok"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success"}"#,
        "\n",
    );

    #[test]
    fn a_stream_gives_the_holst_calls_and_their_errors() {
        assert_eq!(
            parse_stream(STREAM),
            vec![
                ToolUse {
                    tool: "object_write".into(),
                    arguments: json!({"action": "add", "kind": "sticker"}),
                    is_error: true,
                },
                ToolUse {
                    tool: "board".into(),
                    arguments: json!({"action": "ls"}),
                    is_error: false,
                },
            ]
        );
    }

    fn nested_snapshot(parent_of_ship: &str) -> (Snapshot, Snapshot) {
        let member = |id: &str, parent: &str| json!({"id": id, "parentId": parent});
        let group = |id: &str, locked: bool, parent: &str| json!({"id": id, "type": "group", "locked": locked, "parentId": parent});
        let ids: Vec<String> = (1..=8).map(|n| format!("o{n}")).collect();
        let before = Snapshot {
            boards: BTreeMap::from([(
                ROADMAP.to_owned(),
                vec![member("o3", ""), member("o4", "")],
            )]),
            names: vec![],
            threads: vec![],
            ids,
            exports: BTreeMap::new(),
        };
        let objects = vec![
            member("o3", "inner"),
            member("o4", parent_of_ship),
            group("inner", false, "outer"),
            group("outer", true, ""),
        ];
        let after = Snapshot {
            boards: BTreeMap::from([(ROADMAP.to_owned(), objects)]),
            ..before.clone()
        };
        (before, after)
    }

    #[test]
    fn a_locked_group_around_a_new_group_holds_both_members() {
        let check = |parent_of_ship| {
            let (before, after) = nested_snapshot(parent_of_ship);
            locked_group_of_both(&Seen {
                before: &before,
                after: &after,
            })
        };
        assert_eq!(check("outer"), Ok(()));
        assert!(check("elsewhere").is_err());
    }

    async fn broken_checks() -> Vec<String> {
        oracle().await.map_or_else(
            |error| vec![error.to_string()],
            |lines| {
                (lines.len() != TASKS)
                    .then(|| format!("{} tasks proven of {TASKS}", lines.len()))
                    .into_iter()
                    .chain(
                        lines
                            .iter()
                            .filter(|line| !line.proven())
                            .map(|line| format!("{line:?}")),
                    )
                    .collect()
            },
        )
    }

    #[tokio::test]
    async fn the_oracle_proves_every_check() {
        let broken = broken_checks().await;
        assert!(broken.is_empty(), "{}", broken.join("\n"));
    }
}
