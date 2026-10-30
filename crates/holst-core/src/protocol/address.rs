//! Server address rules: where the board socket, backups and storage live.

use reqwest::Url;

use crate::{
    domain::url::BaseUrl,
    error::{Error, Result, board_socket::BoardSocketError, input::InputError},
};

impl BaseUrl {
    /// Where board images are stored: `storage-edge.` before the host, as the
    /// web client loads them; a local mock serves `/storage/` itself.
    #[must_use]
    pub fn storage(&self) -> String {
        let scheme = self.url.scheme();
        self.url
            .domain()
            .filter(|host| host.contains('.') && scheme == "https")
            .map_or_else(
                || format!("{}/storage/", self.raw),
                |host| format!("{scheme}://storage-edge.{host}/storage/"),
            )
    }

    /// Returns the stored name of `address` when it lies in the public storage.
    #[must_use]
    pub fn stored_name<'a>(&self, address: &'a str) -> Option<&'a str> {
        address
            .strip_prefix(self.storage().as_str())
            .filter(|name| !name.contains(['?', '#', '\\', '%']))
            .filter(|name| {
                name.split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
            })
    }

    /// The board document socket: `wss://<host>/hud/ws` (`ws` on a local mock).
    ///
    /// # Errors
    ///
    /// [`crate::error::input::InputError::BaseNotUrl`] when the base cannot
    /// take a socket scheme.
    pub fn board_socket(&self) -> Result<Url> {
        let scheme = match self.url.scheme() {
            "http" => "ws",
            _ => "wss",
        };
        // imperative: `Url` is changed through setters only.
        let mut url = self.url.clone();
        url.set_path("/hud/ws");
        url.set_scheme(scheme).map(|()| url).map_err(|()| {
            Error::Input(InputError::BaseNotUrl {
                base: self.raw.clone(),
            })
        })
    }

    /// A snapshot link from the board server, only on the base host or below
    /// it.
    ///
    /// # Errors
    ///
    /// [`crate::error::input::InputError::NotUrl`] or
    /// [`crate::error::board_socket::BoardSocketError::ForeignLink`] for
    /// another host or plain http.
    pub fn trusted_update_link(&self, link: &str) -> Result<Url> {
        let url = Url::parse(link).map_err(|_| {
            Error::Input(InputError::NotUrl {
                url: link.to_owned(),
            })
        })?;
        let base_host = self.url.host_str().unwrap_or_default();
        let host = url.host_str().unwrap_or_default().to_owned();
        let below = host == base_host || host.ends_with(&format!(".{base_host}"));
        if below && url.scheme() == self.url.scheme() {
            Ok(url)
        } else {
            Err(Error::BoardSocket(BoardSocketError::ForeignLink { host }))
        }
    }
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use crate::domain::url::BaseUrl;

    fn base(raw: &str) -> Option<BaseUrl> {
        raw.parse().ok()
    }

    #[test]
    fn storage_sits_on_the_edge_host_except_on_a_local_mock() {
        assert_eq!(
            base("https://app.holst.so").map(|b| b.storage()).as_deref(),
            Some("https://storage-edge.app.holst.so/storage/")
        );
        assert_eq!(
            base("http://127.0.0.1:8080")
                .map(|b| b.storage())
                .as_deref(),
            Some("http://127.0.0.1:8080/storage/")
        );
    }

    #[test]
    fn stored_name_needs_the_public_storage_and_a_plain_name() {
        let stored = "https://storage-edge.app.holst.so/storage/a1.png";
        let evil = "https://evil.test/storage/a1.png";
        let dots = "https://storage-edge.app.holst.so/storage/../x";
        let base = base("https://app.holst.so");
        assert_eq!(
            base.as_ref().and_then(|b| b.stored_name(stored)),
            Some("a1.png")
        );
        assert_eq!(base.as_ref().and_then(|b| b.stored_name(evil)), None);
        assert_eq!(base.as_ref().and_then(|b| b.stored_name(dots)), None);
    }

    #[test]
    fn board_socket_is_wss_on_https() {
        let socket = base("https://app.holst.so").and_then(|b| b.board_socket().ok());
        assert_eq!(
            socket.as_ref().map(Url::as_str),
            Some("wss://app.holst.so/hud/ws")
        );
    }

    #[test]
    fn update_link_stays_on_the_base_host_and_scheme() {
        let base = base("https://app.holst.so");
        let ok = |link: &str| {
            base.as_ref()
                .is_some_and(|b| b.trusted_update_link(link).is_ok())
        };
        assert!(ok("https://cdns-edge.app.holst.so/gapi/hud/download/b/s"));
        assert!(!ok("https://evil.test/x"));
        assert!(!ok("http://cdns-edge.app.holst.so/x"));
    }
}
