//! The sessions of the fake: who is signed in, the emailed code, logout.

use serde_json::{Value, json};

use crate::fake::{
    ACCEPTED_COOKIES,
    ACCEPTED_TOKEN,
    http::{Parts, Request, Response, found},
};

/// The code every email login accepts, as the CLI's own tests use it.
pub const LOGIN_CODE: &str = "123456";

const STAMP: &str = "2026-10-27T11:35:52.116491Z";

fn session(id: &str, device: &str, location: &Value, current: bool) -> Value {
    json!({
        "id": id,
        "startTime": STAMP,
        "lastActiveAt": STAMP,
        "isCurrent": current,
        "deviceOS": "macOS",
        "deviceName": device,
        "location": location,
    })
}

/// The cookies the server accepts, and the socket tokens it handed out.
#[derive(Debug)]
pub(crate) struct Sessions {
    issued: u32,
    alive: Vec<String>,
    tokens: Vec<String>,
    rotating: bool,
    access_tokens: Vec<String>,
    other_clients: Vec<String>,
}

impl Sessions {
    /// Starts with the accepted session; `access_tokens` are the names of the
    /// personal access tokens and `other_clients` the clients of the other
    /// sessions the account holds.
    pub(crate) fn fixture(
        rotating: bool,
        access_tokens: Vec<String>,
        other_clients: Vec<String>,
    ) -> Self {
        Self {
            issued: 0,
            alive: vec![ACCEPTED_COOKIES.to_owned()],
            tokens: vec![ACCEPTED_TOKEN.to_owned()],
            rotating,
            access_tokens,
            other_clients,
        }
    }

    pub(crate) fn accepts(&self, cookie: &str) -> bool {
        self.alive_in(cookie).is_some()
    }

    pub(crate) fn accepts_token(&self, token: &str) -> bool {
        self.tokens.iter().any(|issued| issued == token)
    }

    /// Answers the email login: the code opens a new session, anything else
    /// is a bad request.
    pub(crate) fn log_in(&mut self, body: &str) -> Response {
        if body.contains(&format!("\"{LOGIN_CODE}\"")) {
            self.issued += 1;
            let number = self.issued;
            let cookie = format!("sessionToken=srv-{number}");
            let token = format!("csrf-{number}");
            self.alive.push(cookie.clone());
            self.tokens.push(token.clone());
            Response {
                status: 200,
                body: json!({ "token": token }).to_string(),
                cookie: Some(format!("{cookie}; Path=/; HttpOnly")),
            }
        } else {
            Response {
                status: 400,
                body: json!({ "errCode": "bad-request" }).to_string(),
                cookie: None,
            }
        }
    }

    /// Replaces the session of `cookie` with a new one and returns the
    /// `set-cookie` value, when the fake rotates and the cookie is alive.
    pub(crate) fn rotate(&mut self, cookie: &str) -> Option<String> {
        let old = self
            .rotating
            .then(|| self.alive_in(cookie).map(str::to_owned))
            .flatten()?;
        self.issued += 1;
        let new = format!("sessionToken=rotated-{}", self.issued);
        self.alive.retain(|alive| *alive != old);
        self.alive.push(new.clone());
        Some(format!("{new}; Path=/; HttpOnly"))
    }

    /// Ends the session of the request's cookie only.
    pub(crate) fn end(&mut self, cookie: &str) {
        let ended = self.alive_in(cookie).map(str::to_owned);
        self.alive.retain(|alive| Some(alive) != ended.as_ref());
    }

    /// Answers the lists of the account page: the sessions and the tokens.
    pub(crate) fn respond(&self, request: &Request) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "session", "list"]) => {
                Some(found(&self.sessions(&request.cookie)))
            }
            ("GET", ["", "gapi", "personal_access_token", "list"]) => {
                Some(found(&self.token_names()))
            }
            _ => None,
        }
    }

    /// The live sessions as the live API lists them (a bare array), the one
    /// of `cookie` marked, then the filled others.
    fn sessions(&self, cookie: &str) -> Value {
        let own = self.alive_in(cookie);
        let live = self.alive.iter().enumerate().map(|(index, alive)| {
            session(
                &format!("session-{index}"),
                "holst-rs",
                &json!({ "city": "Moscow", "country": "Russia" }),
                own == Some(alive.as_str()),
            )
        });
        let others = self
            .other_clients
            .iter()
            .enumerate()
            .map(|(index, client)| {
                session(
                    &format!("session-other-{index}"),
                    client,
                    &json!({ "country": "Russia" }),
                    false,
                )
            });
        Value::Array(live.chain(others).collect())
    }

    /// The active tokens by name as the live API lists them (a bare array);
    /// a secret is shown once, when made, never again.
    fn token_names(&self) -> Value {
        Value::Array(
            self.access_tokens
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    json!({
                        "id": format!("token-{index}"),
                        "name": name,
                        "tokenPrefix": format!("holst_pat_{index:02}"),
                        "scopes": ["organization:audit_events_read"],
                        "status": "active",
                        "createdAt": STAMP,
                        "expiresAt": null,
                        "revokedAt": null,
                        "lastUsedAt": null,
                    })
                })
                .collect(),
        )
    }

    fn alive_in(&self, cookie: &str) -> Option<&str> {
        self.alive
            .iter()
            .map(String::as_str)
            .find(|alive| cookie.split(';').any(|pair| pair.trim() == *alive))
    }
}
