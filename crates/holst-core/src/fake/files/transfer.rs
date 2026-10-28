//! Updates of boards too big for the socket, uploaded and sent as a link.

use serde_json::json;

use crate::fake::http::{Parts, Request, Response, found, refused};

/// What a download of an uploaded update finds.
pub(crate) enum Download {
    Found(Vec<u8>),
    Missing,
}

#[derive(Debug, Default)]
pub(crate) struct Transfers {
    /// The updates uploaded, by `board/update`, the form of the link.
    updates: Vec<(String, Vec<u8>)>,
}

impl Transfers {
    /// Keeps the update `updateId` of the board `documentName` that the
    /// request carries.
    pub(crate) fn store(
        &mut self,
        request: &Request,
        exists: impl Fn(&str) -> bool,
    ) -> Response {
        let parts = Parts::of(request);
        match (parts.query("documentName"), parts.query("updateId")) {
            (Some(board), Some(update)) if exists(board) => {
                let link = format!("{board}/{update}");
                self.updates.retain(|(known, _)| *known != link);
                self.updates.push((link, request.bytes.clone()));
                found(&json!({}))
            }
            (Some(_), Some(_)) => refused(404, "not-found"),
            _ => refused(400, "bad-request"),
        }
    }

    /// The update a link names.
    pub(crate) fn uploaded(&self, link: &str) -> Option<Vec<u8>> {
        self.updates
            .iter()
            .find(|(known, _)| known == link)
            .map(|(_, bytes)| bytes.clone())
    }

    /// Answers a download, which needs no session; none for any other request.
    pub(crate) fn download(&self, request: &Request) -> Option<Download> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "hud", "download", board, update]) => Some(
                self.uploaded(&format!("{board}/{update}"))
                    .map_or(Download::Missing, Download::Found),
            ),
            _ => None,
        }
    }
}
