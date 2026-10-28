//! Board lookup by name end to end against the local delayed mock.

#[path = "../tests/common/mod.rs"] mod common;
mod fixtures;

use std::{
    collections::HashMap,
    io,
    sync::{Arc, LazyLock},
    time::Duration,
};

use divan::Bencher;
use holst_core::remote::client::Client;
use tokio::runtime::Runtime;

#[derive(Debug, thiserror::Error)]
enum SetupError {
    #[error("no async runtime: {0}")]
    Runtime(io::ErrorKind),
    #[error("mock server: {0}")]
    Mock(#[from] io::Error),
    #[error(transparent)]
    Holst(#[from] holst_core::Error),
}

fn main() -> Result<(), SetupError> {
    client(1, Duration::ZERO)?;
    divan::main();
    Ok(())
}

static RT: LazyLock<io::Result<Runtime>> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
});

fn connect(
    workspaces: usize,
    latency: Duration,
) -> Result<(&'static Runtime, Client), SetupError> {
    Ok((rt()?, client(workspaces, latency)?))
}

fn rt() -> Result<&'static Runtime, SetupError> {
    RT.as_ref().map_err(|err| SetupError::Runtime(err.kind()))
}

const TARGET: &str = "Целевая доска";
const BOARDS_PER_WORKSPACE: usize = 500;

fn client(workspaces: usize, latency: Duration) -> Result<Client, SetupError> {
    let user = (
        "/gapi/current_user".to_string(),
        fixtures::current_user(workspaces).to_string().into(),
    );
    let trees = (0..workspaces).map(|w| {
        let target = (w == workspaces - 1).then_some(TARGET);
        (
            format!("/gapi/workspace/{}/tree", fixtures::uuid(0, w)),
            fixtures::tree(w, BOARDS_PER_WORKSPACE, target)
                .to_string()
                .into(),
        )
    });
    let routes: HashMap<String, Arc<str>> =
        std::iter::once(user).chain(trees).collect();
    let mock = rt()?.block_on(common::serve(latency, move |req| {
        let route = req
            .path
            .split_once('?')
            .map_or(&*req.path, |(route, _)| route);
        routes.get(route).map_or_else(
            || (404, String::new(), "{}".into()),
            |body| {
                (
                    200,
                    "content-type: application/json\r\n".into(),
                    Arc::clone(body),
                )
            },
        )
    }));
    Ok(mock?.client("bench-token")?)
}

const WORKSPACES: &[usize] = &[1, 5, 20];
const RTT: Duration = Duration::from_millis(20);

#[divan::bench(args = WORKSPACES, sample_count = 10, sample_size = 1)]
fn resolve_board(bencher: Bencher<'_, '_>, workspaces: usize) {
    let setup = connect(workspaces, RTT);
    bencher.bench(|| {
        setup.as_ref().map(|(rt, client)| {
            rt.block_on(holst_core::command::lookup::board_id(client, TARGET, None))
        })
    });
}
