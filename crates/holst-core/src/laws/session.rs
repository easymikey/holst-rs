use std::collections::{BTreeMap, BTreeSet};

use proptest::{
    collection::vec,
    prelude::{Just, Strategy, prop_assert, prop_assert_eq, proptest},
    sample::select,
};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    domain::{
        cookie::{self, CookieChange, CookieJar},
        session,
        session::Session,
        text::joined,
        time::Timestamp,
    },
    laws::{config, cookie_name, cookie_value, jar, jar_pairs, parsed_jar, token},
};

fn expected_after(
    jar: &BTreeMap<String, String>,
    fresh: &[(String, String)],
    name: &str,
) -> Option<String> {
    fresh
        .iter()
        .rev()
        .find(|(fresh_name, _)| fresh_name == name)
        .map_or_else(
            || jar.get(name).cloned(),
            |(_, value)| Some(value.clone()).filter(|value| !value.is_empty()),
        )
}

/// An empty value clears the cookie, any other sets it.
fn change(name: &str, value: &str) -> CookieChange {
    let name = name.to_owned();
    match value {
        "" => CookieChange::Cleared { name },
        _ => CookieChange::Set {
            name,
            value: value.to_owned(),
        },
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn cookie_jar_round_trips_through_header(jar in jar()) {
        prop_assert_eq!(parsed_jar(jar.header().expose_secret()), jar);
    }

    /// The order of cookies in a stored line does not change the header.
    #[test]
    fn cookie_jar_header_does_not_depend_on_order(
        shuffled in jar_pairs().prop_flat_map(|jar| {
            let pairs: Vec<_> = jar.into_iter().collect();
            (Just(pairs.clone()), Just(pairs).prop_shuffle())
        }),
    ) {
        let (sorted, shuffled) = shuffled;
        let line = |pairs: &[(String, String)]| {
            joined(pairs.iter().map(|(name, value)| format!("{name}={value}")), "; ").to_string()
        };
        prop_assert_eq!(
            parsed_jar(&line(&shuffled)).header().expose_secret().to_owned(),
            parsed_jar(&line(&sorted)).header().expose_secret().to_owned()
        );
    }

    /// From `Set-Cookie` the name and value are taken; attributes are dropped.
    #[test]
    fn set_cookie_keeps_name_and_value(
        name in cookie_name(),
        value in cookie_value().prop_filter("a cleared cookie has no value", |value| !value.is_empty()),
        attributes in vec(select(&["Path=/", "HttpOnly", "Secure", "Max-Age=60", "SameSite=Lax"][..]), 0..4),
    ) {
        let raw = std::iter::once(format!("{name}={value}"))
            .chain(attributes.iter().map(|a| (*a).to_owned()))
            .collect::<Vec<_>>()
            .join("; ");
        prop_assert_eq!(
            cookie::parse_set_cookie(&raw, Timestamp::from_millis(0)),
            Some(CookieChange::Set { name, value })
        );
    }

    /// An `Expires` year past 9999 is no date: the cookie stays, nothing
    /// overflows; every other year is read without a panic.
    #[test]
    fn expires_year_never_overflows(
        far in "[1-9][0-9]{4,40}",
        any_year in "[0-9]{1,40}",
        now in proptest::prelude::any::<u64>(),
    ) {
        let read = |year: &str| {
            cookie::parse_set_cookie(
                &format!("sid=x; Expires=Thu, 01 Jan {year} 00:00:00 GMT"),
                Timestamp::from_millis(now),
            )
        };
        prop_assert_eq!(
            read(&far),
            Some(CookieChange::Set { name: "sid".into(), value: "x".into() })
        );
        prop_assert!(read(&any_year).is_some());
    }

    /// The last fresh value wins, an empty one clears, others stay.
    #[test]
    fn set_cookies_override_and_empty_ones_clear(
        jar in jar_pairs(),
        fresh in vec((cookie_name(), cookie_value()), 0..6),
    ) {
        let after = jar.clone().into_iter().collect::<CookieJar>().merge(
            fresh.iter().map(|(name, value)| change(name, value)),
        );
        let names: BTreeSet<_> =
            jar.keys().chain(fresh.iter().map(|(name, _)| name)).collect();
        let expected: CookieJar = names
            .into_iter()
            .filter_map(|name| Some((name.clone(), expected_after(&jar, &fresh, name)?)))
            .collect();
        prop_assert_eq!(after, expected);
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn session_round_trips_through_saved_login(token in token(), jar in jar()) {
        let session = Session::with_cookies(SecretString::from(token), jar);
        prop_assert_eq!(
            session.to_saved_login().expose_secret().parse::<Session>().ok(),
            Some(session)
        );
    }

    /// A bare token (an old saved login) is read with spaces trimmed.
    #[test]
    fn bare_token_is_read_trimmed(token in "[A-Za-z0-9_.-]{1,48}", pad in "[ \t\n]{0,3}") {
        prop_assert_eq!(
            format!("{pad}{token}{pad}").parse::<Session>().ok(),
            Some(Session::new(SecretString::from(token)))
        );
    }

    #[test]
    fn redacted_token_depends_only_on_its_length(token in "[A-Za-z0-9_-]{9,64}") {
        let blank = "x".repeat(token.chars().count());
        prop_assert_eq!(session::redact(&token), session::redact(&blank));
    }
}
