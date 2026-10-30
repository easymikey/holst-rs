//! The workspace list and trees of an account on disk, beside its boards.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    domain::{account::Workspace, id::is_uuid},
    effect::{
        cache::{AccountCache, CachedTree, Forget},
        settings::CacheSettings,
    },
    error::Result,
    storage::{
        cache::{
            TREES_DIR,
            WORKSPACES_FILE,
            files::{is_host_key, layout},
        },
        file::{private_dir, remove_any, replace_private, tighten, write_error},
    },
};

/// Beside the trees: when the account's trees were last forgotten.
const FORGOTTEN_FILE: &str = "forgotten-at";

/// The workspace lists and trees of every account under one server's folder.
#[derive(Debug, Clone)]
pub struct AccountDisk {
    root: PathBuf,
    host: String,
}

impl AccountDisk {
    /// The store on the settings' server; `None` when the cache is off.
    #[must_use]
    pub fn from_settings(settings: &CacheSettings) -> Option<Self> {
        let root = settings.root.clone()?;
        is_host_key(&settings.host).then(|| Self {
            root,
            host: settings.host.clone(),
        })
    }

    fn dir(&self, account: &str) -> Option<PathBuf> {
        let account = account.to_ascii_lowercase();
        is_uuid(&account).then(|| layout(&self.root).join(&self.host).join(account))
    }

    fn tree_file(&self, account: &str, workspace: &str) -> Option<PathBuf> {
        let workspace = workspace.to_ascii_lowercase();
        is_uuid(&workspace)
            .then(|| {
                self.dir(account)
                    .map(|dir| dir.join(TREES_DIR).join(workspace))
            })
            .flatten()
    }

    fn forgotten_at(&self, account: &str) -> Option<u64> {
        let bytes = std::fs::read(self.dir(account)?.join(FORGOTTEN_FILE)).ok()?;
        String::from_utf8(bytes).ok()?.trim().parse().ok()
    }

    /// Notes that trees were forgotten at `at`, or at the later time noted.
    fn mark_forgotten(&self, account: &str, at: u64) -> Result<()> {
        let at = at.max(self.forgotten_at(account).unwrap_or_default());
        self.dir(account).map_or(Ok(()), |dir| {
            self.write(&dir.join(FORGOTTEN_FILE), &[at.to_string().as_bytes()])
        })
    }

    fn write(&self, path: &Path, chunks: &[&[u8]]) -> Result<()> {
        let parent = path.parent().unwrap_or(path);
        private_dir(parent).map_err(|err| write_error(parent, err))?;
        let layout = layout(&self.root);
        parent
            .ancestors()
            .take_while(|dir| dir.starts_with(&layout))
            .try_for_each(|dir| tighten(dir).map_err(|err| write_error(dir, err)))?;
        replace_private(path, chunks)
    }
}

impl AccountCache for AccountDisk {
    fn workspaces(&self, account: &str) -> Option<Vec<Workspace>> {
        let bytes = std::fs::read(self.dir(account)?.join(WORKSPACES_FILE)).ok()?;
        let items: Vec<Value> = serde_json::from_slice(&bytes).ok()?;
        items
            .iter()
            .map(|item| {
                Some(Workspace {
                    id: item.get("id")?.as_str()?.to_owned(),
                    name: item.get("name")?.as_str()?.to_owned(),
                })
            })
            .collect()
    }

    fn keep_workspaces(&self, account: &str, workspaces: &[Workspace]) -> Result<()> {
        self.dir(account).map_or(Ok(()), |dir| {
            let list: Vec<Value> = workspaces
                .iter()
                .map(|space| json!({ "id": space.id, "name": space.name }))
                .collect();
            let bytes = Value::Array(list).to_string();
            self.write(&dir.join(WORKSPACES_FILE), &[bytes.as_bytes()])
        })
    }

    fn tree(&self, account: &str, workspace: &str) -> Option<CachedTree> {
        let bytes = std::fs::read(self.tree_file(account, workspace)?).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let (first, text) = text.split_once('\n')?;
        Some(CachedTree {
            text: text.to_owned(),
            read_at: first.parse().ok()?,
        })
    }

    fn keep_tree(
        &self,
        account: &str,
        workspace: &str,
        text: &str,
        read_at: u64,
    ) -> Result<()> {
        let stale = self
            .forgotten_at(account)
            .is_some_and(|forgotten| forgotten > read_at);
        self.tree_file(account, workspace)
            .filter(|_| !stale)
            .map_or(Ok(()), |path| {
                let head = format!("{read_at}\n");
                self.write(&path, &[head.as_bytes(), text.as_bytes()])
            })
    }

    fn forget(&self, account: &str, what: Forget<'_>, at: u64) -> Result<()> {
        self.dir(account).map_or(Ok(()), |dir| {
            let list = dir.join(WORKSPACES_FILE);
            let trees = dir.join(TREES_DIR);
            match what {
                Forget::Workspaces => remove_any(&list),
                Forget::Trees => self
                    .mark_forgotten(account, at)
                    .and_then(|()| remove_any(&trees)),
                Forget::Tree(workspace) => {
                    self.mark_forgotten(account, at).and_then(|()| {
                        self.tree_file(account, workspace)
                            .map_or(Ok(()), |path| remove_any(&path))
                    })
                }
                Forget::All => self
                    .mark_forgotten(account, at)
                    .and_then(|()| remove_any(&list).and(remove_any(&trees))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use std::{fs, io, path::PathBuf};

    use rstest::rstest;

    use crate::{
        domain::account::Workspace,
        effect::{
            cache::{AccountCache, CachedTree, Forget},
            settings::CacheSettings,
        },
        error::{Error, Result, local_data::LocalDataError},
        storage::cache::account::AccountDisk,
    };

    const ALICE: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const BOB: &str = "bbbbbbbb-0000-4000-8000-000000000002";
    const ONE: &str = "11111111-1111-4000-8000-000000000001";
    const TWO: &str = "22222222-2222-4000-8000-000000000002";

    fn settings(test: &str, host: &str) -> io::Result<CacheSettings> {
        let dir = std::env::temp_dir()
            .join(format!("holst-account-{test}-{}", std::process::id()));
        fs::remove_dir_all(&dir).or_else(|err| {
            (err.kind() == io::ErrorKind::NotFound)
                .then_some(())
                .ok_or(err)
        })?;
        Ok(CacheSettings {
            root: Some(dir.clone()),
            roots: vec![dir],
            host: host.into(),
            max_bytes: 1 << 20,
            trust_millis: 0,
        })
    }

    fn store(test: &str) -> Result<(AccountDisk, PathBuf)> {
        let settings = settings(test, "app.holst.so")?;
        let root = settings.root.clone().unwrap_or_default();
        let store = AccountDisk::from_settings(&settings)
            .ok_or(Error::LocalData(LocalDataError::NoHome))?;
        Ok((store, root))
    }

    fn space(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn a_kept_list_and_tree_read_back() -> Result<()> {
        let (store, _) = store("readback")?;
        let list = vec![space(ONE, "One"), space(TWO, "Two")];
        store.keep_workspaces(ALICE, &list)?;
        store.keep_tree(ALICE, ONE, "line\nlines", 42)?;
        assert_eq!(store.workspaces(ALICE), Some(list));
        assert_eq!(
            store.tree(ALICE, ONE),
            Some(CachedTree {
                text: "line\nlines".into(),
                read_at: 42
            })
        );
        assert_eq!(store.tree(ALICE, TWO), None);
        Ok(())
    }

    #[test]
    fn another_account_or_host_reads_none() -> Result<()> {
        let (store, root) = store("others")?;
        store.keep_workspaces(ALICE, &[space(ONE, "One")])?;
        store.keep_tree(ALICE, ONE, "t", 1)?;
        assert_eq!(store.workspaces(BOB), None);
        assert_eq!(store.tree(BOB, ONE), None);
        let mut other = settings("others", "other.example")?;
        other.root = Some(root);
        let other = AccountDisk::from_settings(&other)
            .ok_or(Error::LocalData(LocalDataError::NoHome))?;
        assert_eq!(other.workspaces(ALICE), None);
        assert_eq!(other.tree(ALICE, ONE), None);
        Ok(())
    }

    #[test]
    fn a_non_uuid_keeps_nothing() -> Result<()> {
        let (store, root) = store("non-uuid")?;
        store.keep_workspaces("../x", &[space(ONE, "One")])?;
        store.keep_tree(ALICE, "../x", "t", 1)?;
        store.keep_tree("alice", ONE, "t", 1)?;
        assert!(!root.exists(), "something was written");
        Ok(())
    }

    #[test]
    fn forget_drops_only_what_it_names() -> Result<()> {
        let (store, _) = store("forget")?;
        let fill = || -> Result<()> {
            store.keep_workspaces(ALICE, &[space(ONE, "One")])?;
            store.keep_tree(ALICE, ONE, "a", 1)?;
            store.keep_tree(ALICE, TWO, "b", 2)
        };
        fill()?;
        store.forget(ALICE, Forget::Workspaces, 1)?;
        assert!(store.workspaces(ALICE).is_none() && store.tree(ALICE, ONE).is_some());
        fill()?;
        store.forget(ALICE, Forget::Tree(ONE), 1)?;
        assert!(store.tree(ALICE, ONE).is_none() && store.tree(ALICE, TWO).is_some());
        assert!(store.workspaces(ALICE).is_some());
        fill()?;
        store.forget(ALICE, Forget::Trees, 1)?;
        assert!(store.tree(ALICE, ONE).is_none() && store.tree(ALICE, TWO).is_none());
        assert!(store.workspaces(ALICE).is_some());
        fill()?;
        store.forget(ALICE, Forget::All, 1)?;
        assert!(store.workspaces(ALICE).is_none() && store.tree(ALICE, TWO).is_none());
        Ok(())
    }

    #[rstest]
    #[case::one_tree("stale-one", Forget::Tree(ONE))]
    #[case::all_trees("stale-trees", Forget::Trees)]
    #[case::everything("stale-all", Forget::All)]
    fn a_tree_read_that_started_before_a_forget_is_not_kept(
        #[case] name: &str,
        #[case] what: Forget<'_>,
    ) -> Result<()> {
        let (store, _) = store(name)?;
        store.forget(ALICE, what, 10)?;
        store.keep_tree(ALICE, ONE, "old", 9)?;
        assert_eq!(store.tree(ALICE, ONE), None);
        store.keep_tree(ALICE, ONE, "new", 10)?;
        assert_eq!(
            store.tree(ALICE, ONE).map(|tree| tree.text),
            Some("new".into())
        );
        Ok(())
    }

    #[test]
    fn a_forget_of_the_list_leaves_no_mark_on_trees() -> Result<()> {
        let (store, _) = store("list-forget")?;
        store.forget(ALICE, Forget::Workspaces, 10)?;
        store.keep_tree(ALICE, ONE, "old", 9)?;
        assert!(store.tree(ALICE, ONE).is_some());
        Ok(())
    }

    #[test]
    fn the_cache_off_has_no_store() -> Result<()> {
        let mut off = settings("off", "app.holst.so")?;
        off.root = None;
        assert!(AccountDisk::from_settings(&off).is_none());
        Ok(())
    }

    #[test]
    fn a_truncated_file_reads_as_none() -> Result<()> {
        let (store, root) = store("truncated")?;
        store.keep_workspaces(ALICE, &[space(ONE, "One")])?;
        store.keep_tree(ALICE, ONE, "text", 7)?;
        let dir = root.join("v2/app.holst.so").join(ALICE);
        fs::write(dir.join("workspaces.json"), "[{\"id\":")?;
        fs::write(dir.join("trees").join(ONE), "12")?;
        assert_eq!(store.workspaces(ALICE), None);
        assert_eq!(store.tree(ALICE, ONE), None);
        Ok(())
    }
}
