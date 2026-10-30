//! The sessions and personal access tokens of an account, as the lists read.

use serde::{Deserialize, Serialize};

/// Where the server placed a session; it may know only the country.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

impl Location {
    fn text(&self) -> Option<String> {
        let text = [self.city.as_deref(), self.country.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        Some(text).filter(|text| !text.is_empty())
    }
}

/// A login of the account on some device. Dates are RFC 3339 as sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub id: String,
    #[serde(alias = "deviceName", skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(alias = "deviceOS", skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(alias = "startTime", skip_serializing_if = "Option::is_none")]
    pub started: Option<String>,
    #[serde(alias = "lastActiveAt", skip_serializing_if = "Option::is_none")]
    pub last_active: Option<String>,
    #[serde(alias = "isCurrent")]
    pub current: bool,
}

impl Session {
    /// `device · OS · city, country · last active <date>`, marked when it is
    /// the session that asked.
    #[must_use]
    pub fn line(&self) -> String {
        let line = [
            self.device.clone(),
            self.os.clone(),
            self.location.as_ref().and_then(Location::text),
            self.last_active
                .as_deref()
                .map(|stamp| format!("last active {}", day(stamp))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let mark = self.current.then_some(" (this session)");
        line + mark.unwrap_or_default()
    }
}

/// A personal access token of the account; the secret is never read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Token {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(alias = "tokenPrefix", skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(alias = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(alias = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(alias = "lastUsedAt", skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

impl Token {
    /// `name · prefix… · scope · expires <date> · last used <date>`.
    #[must_use]
    pub fn line(&self) -> String {
        [
            self.name.clone(),
            self.prefix.as_ref().map(|prefix| format!("{prefix}…")),
            Some(self.scopes.join(", ")).filter(|scopes| !scopes.is_empty()),
            self.expires
                .as_deref()
                .map(|stamp| format!("expires {}", day(stamp))),
            self.last_used
                .as_deref()
                .map(|stamp| format!("last used {}", day(stamp))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

fn day(stamp: &str) -> &str {
    stamp.split_once('T').map_or(stamp, |(day, _)| day)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use super::{Session, Token};

    #[test]
    fn a_session_reads_the_live_names_and_a_partial_location() -> serde_json::Result<()>
    {
        let session: Session = serde_json::from_str(
            r#"{"id":"s1","deviceName":"Chrome","deviceOS":"macOS","isCurrent":true,
            "lastActiveAt":"2026-10-27T12:40:26.381268Z","startTime":"2026-10-27T11:35:52Z",
            "location":{"country":"Russia"},"extra":1}"#,
        )?;
        assert_eq!(
            session.line(),
            "Chrome · macOS · Russia · last active 2026-10-27 (this session)"
        );
        Ok(())
    }

    #[test]
    fn a_token_without_expiry_or_use_keeps_its_other_parts() -> serde_json::Result<()> {
        let token: Token = serde_json::from_str(
            r#"{"id":"t1","name":"ci","tokenPrefix":"holst_pat_a1","scopes":["organization:audit_events_read"],"expiresAt":null}"#,
        )?;
        assert_eq!(
            token.line(),
            "ci · holst_pat_a1… · organization:audit_events_read"
        );
        Ok(())
    }
}
