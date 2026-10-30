//! Routes each connection to the socket or to an endpoint module.

use std::{
    io,
    sync::{Arc, PoisonError},
};

use serde_json::json;
use tokio::net::{TcpListener, TcpStream};

use crate::fake::{
    Shared,
    State,
    files::Download,
    http::{
        Request,
        Response,
        found,
        read_request,
        refused,
        write_body,
        write_response,
    },
    socket,
};

pub(crate) async fn accept(listener: TcpListener, state: Shared) {
    // imperative: accepts for the life of the test
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(respond(stream, Arc::clone(&state)));
    }
}

async fn respond(mut stream: TcpStream, state: Shared) {
    // A client that hung up shows itself in the scenario.
    let handshake = socket::is_handshake(&stream, &state)
        .await
        .then(|| Arc::clone(&state));
    match handshake {
        Some(shared) => drop(socket::serve(stream, shared).await),
        None => drop(serve(&mut stream, &state).await),
    }
}

async fn serve(stream: &mut TcpStream, state: &Shared) -> io::Result<()> {
    let request = read_request(stream).await?;
    match answer(state, &request) {
        Answer::Update(bytes) => {
            write_body(stream, (200, "application/octet-stream"), "", &bytes).await
        }
        Answer::Json(reply) => write_response(stream, &reply).await,
    }
}

enum Answer {
    Update(Vec<u8>),
    Json(Response),
}

fn answer(state: &Shared, request: &Request) -> Answer {
    let mut locked = state.lock().unwrap_or_else(PoisonError::into_inner);
    locked.requests.push(format!(
        "{} {}",
        request.method,
        request.path.split('?').next().unwrap_or_default()
    ));
    // An uploaded update needs no session, like the storage; a snapshot does.
    let signed_in = locked.sessions.accepts(&request.cookie);
    match locked.files.download(request, signed_in) {
        Some(Download::Found(bytes)) => Answer::Update(bytes),
        Some(Download::Missing) => Answer::Json(refused(404, "not-found")),
        None => Answer::Json(route(&mut locked, request)),
    }
}

fn route(state: &mut State, request: &Request) -> Response {
    // The storage is public: a download carries neither token nor cookies.
    match request
        .path
        .strip_prefix("/storage/")
        .filter(|_| request.method == "GET")
    {
        Some(name) => state
            .tree
            .stored(name)
            .unwrap_or_else(|| refused(404, "not-found")),
        None => api(state, request),
    }
}

/// Answers the API, then rotates the session cookie when the fixture asks for
/// it.
fn api(state: &mut State, request: &Request) -> Response {
    let reply = failing(state, request).unwrap_or_else(|| respond_api(state, request));
    // A logout ends the session; there is nothing to rotate.
    let rotated = (request.path != "/gapi/logout")
        .then(|| state.sessions.rotate(&request.cookie))
        .flatten();
    Response {
        cookie: reply.cookie.or(rotated),
        ..reply
    }
}

/// Answers with the status a test set for the path, whatever the session.
fn failing(state: &State, request: &Request) -> Option<Response> {
    let path = request.path.split('?').next().unwrap_or_default();
    state
        .failing
        .iter()
        .find(|(failed, _)| failed == path)
        .map(|(_, status)| {
            refused(
                *status,
                match status {
                    429 => "too-many-requests",
                    _ => "unavailable",
                },
            )
        })
}

fn respond_api(state: &mut State, request: &Request) -> Response {
    let signed_in = state.sessions.accepts(&request.cookie);
    let unauthorized = || refused(401, "unauthorized");
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/gapi/login/get_public_options") => {
            found(&json!({ "hasAccount": true, "viaTemporaryPassword": true }))
        }
        ("POST", "/gapi/send_temporary_password") => found(&json!({})),
        ("POST", "/gapi/login/with_email") => state.sessions.log_in(&request.body),
        ("POST", "/gapi/logout") if signed_in => {
            state.sessions.end(&request.cookie);
            found(&json!({}))
        }
        ("GET", "/gapi/current_user") if signed_in => {
            found(&state.workspaces.current_user())
        }
        // The socket handshake without the session cookie never reaches the socket.
        ("GET", "/gapi/current_user" | "/hud/ws") => unauthorized(),
        (_, _) if !signed_in => unauthorized(),
        (method, path) => {
            let ids = state.workspaces.ids();
            state
                .tree
                .respond(request, &ids)
                .map(|reply| state.access.shown_on(request, reply))
                .or_else(|| state.catalog.respond(request))
                .or_else(|| {
                    state
                        .access
                        .respond(request, |id| state.tree.owner_of(id).is_some())
                })
                .or_else(|| {
                    state
                        .comments
                        .respond(request, |id| state.tree.board(id).is_some())
                })
                .or_else(|| {
                    state
                        .files
                        .respond(request, |id| state.tree.board(id).is_some())
                })
                .or_else(|| state.sessions.respond(request))
                .or_else(|| state.workspaces.respond(request))
                .unwrap_or_else(|| {
                    state.unanswered.push(format!("{method} {path}"));
                    refused(404, "not-found")
                })
        }
    }
}
