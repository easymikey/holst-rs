//! The settings of a run from the flags, else the environment; built in `main`.

use std::{ffi::OsString, path::PathBuf};

use clap::Args;
use holst_core::{
    domain::url::{BaseUrl, DEFAULT_BASE},
    effect::settings::{
        BASE_URL_VAR,
        CacheSettings,
        HOME_VAR,
        LogSettings,
        SessionLocation,
        Settings,
        user_home,
    },
};

/// The API address and where the login is kept: flags, else the environment.
#[derive(Debug, Clone, Args)]
pub(crate) struct SettingsArgs {
    #[arg(long, global = true, help = "The Holst server; https://app.holst.so by default", value_name = "url", env = BASE_URL_VAR, default_value = DEFAULT_BASE)]
    base_url: BaseUrl,
    /// Keep the saved login in this directory, not in the keychain
    #[arg(long, global = true, value_name = "dir", env = HOME_VAR)]
    home: Option<PathBuf>,
    /// Read boards from the server, not from the board cache
    #[arg(long = "no-cache", global = true)]
    no_cache: bool,
}

impl SettingsArgs {
    #[must_use]
    pub(crate) fn into_settings(
        self,
        read_variable: impl Fn(&str) -> Option<OsString>,
    ) -> (Settings, Option<holst_core::Error>) {
        let location = SessionLocation::new(self.home, user_home(&read_variable));
        let (cache, cache_error) =
            CacheSettings::from_variables(&read_variable, &location, &self.base_url);
        let settings = Settings {
            base_url: self.base_url,
            cache: if self.no_cache { cache.off() } else { cache },
            location,
            log: LogSettings::from_variables(read_variable),
        };
        (settings, cache_error)
    }
}

#[cfg(test)]
mod tests {
    use holst_core::effect::settings::{SessionLocation, Settings};

    use crate::args::settings::SettingsArgs;

    #[derive(clap::Parser)]
    struct Probe {
        #[command(flatten)]
        settings: SettingsArgs,
    }

    /// The settings `words` give, without the environment.
    fn parsed(words: &[&str]) -> Option<Settings> {
        <Probe as clap::Parser>::try_parse_from(words)
            .ok()
            .map(|probe| probe.settings.into_settings(|_| None).0)
    }

    /// `--home` names a directory location; without it, the system stores.
    #[test]
    fn holst_home_picks_the_directory() {
        let with = parsed(&["t", "--home", "/tmp/h"]);
        let location = with.as_ref().map(|settings| settings.location.clone());
        assert_eq!(location, Some(SessionLocation::Dir("/tmp/h".into())));
        assert_eq!(
            location.and_then(|location| location.file()),
            Some("/tmp/h/session".into())
        );
        let cache = with.and_then(|settings| settings.cache.root);
        assert_eq!(cache, Some("/tmp/h/cache".into()));
    }

    #[test]
    fn base_url_flag_is_checked() {
        let settings = parsed(&["t", "--base-url", "http://127.0.0.1:9"]);
        assert_eq!(
            settings.map(|settings| settings.base_url.as_str().to_owned()),
            Some("http://127.0.0.1:9".into())
        );
        assert!(parsed(&["t", "--base-url", "not a url"]).is_none());
    }

    /// `--no-cache` turns the cache off; logout still sees it.
    #[test]
    fn no_cache_turns_the_cache_off() {
        let off = parsed(&["t", "--home", "/tmp/h", "--no-cache"]);
        assert_eq!(
            off.map(|settings| (settings.cache.root, settings.cache.roots)),
            Some((None, vec!["/tmp/h/cache".into()]))
        );
    }

    /// A bad cache variable is a warning, not a failure: the cache is off.
    #[test]
    fn a_bad_cache_variable_is_no_failure() {
        let probe =
            <Probe as clap::Parser>::try_parse_from(["t", "--home", "/tmp/h"]).ok();
        let read_variable =
            |name: &str| (name == "HOLST_CACHE_MAX").then(|| "lots".into());
        let checked = probe.map(|probe| probe.settings.into_settings(read_variable));
        let (settings, cache_error) = checked.unzip();
        assert_eq!(settings.map(|settings| settings.cache.root), Some(None));
        assert!(cache_error.flatten().is_some());
    }
}
