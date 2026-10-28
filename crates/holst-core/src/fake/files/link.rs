//! The preview of a web page that a link card shows.

use serde_json::json;

use crate::fake::http::{Response, found, refused};

/// What the server finds when it fetches a page.
#[derive(Debug, Clone)]
pub(crate) enum PageOutcome {
    Title(String),
    Fails,
}

/// A page the fake knows when it starts.
#[derive(Debug, Clone)]
pub(crate) struct FixturePage {
    pub(crate) url: String,
    pub(crate) outcome: PageOutcome,
}

#[derive(Debug, Default)]
pub(crate) struct Pages {
    known: Vec<FixturePage>,
}

impl Pages {
    pub(crate) fn fixture(pages: &[FixturePage]) -> Self {
        Self {
            known: pages.to_vec(),
        }
    }

    /// Answers the preview of the page at `link`: the title it was filled
    /// with, nothing for a page it was not, an error for a page that fails.
    pub(crate) fn preview(&self, link: Option<&str>) -> Response {
        link.map_or_else(
            || refused(400, "bad-request"),
            |url| {
                self.known.iter().find(|page| page.url == url).map_or_else(
                    || found(&json!({})),
                    |page| match &page.outcome {
                        PageOutcome::Title(title) => found(&json!({ "title": title })),
                        PageOutcome::Fails => Response {
                            status: 400,
                            body: json!({
                                "errCode": "bad-request",
                                "msg": "",
                                "vars": null,
                            })
                            .to_string(),
                            cookie: None,
                        },
                    },
                )
            },
        )
    }
}
