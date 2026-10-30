use std::collections::BTreeSet;

use proptest::{
    collection::{btree_map, vec},
    option,
    prelude::{
        Just,
        Strategy,
        any,
        prop_assert,
        prop_assert_eq,
        prop_assume,
        prop_oneof,
        proptest,
    },
    sample::select,
};
use serde_json::{Map, Value, json};

use crate::{
    domain::{
        account::{Workspace, account_workspaces},
        backup::Backup,
        id::{FolderId, WorkspaceId},
        login::Email,
        node::{CreationStamp, NewNode, NewParent, NodeBrief, NodeKind, copy_name},
        template::Template,
        time::Timestamp,
    },
    laws::{LAST_MS, board_name, config, json_value, uuid},
    protocol::upload::multipart_body,
    remote::api::channel::CreateChannel,
};

proptest! {
    #![proptest_config(config())]

    /// A channel kind parses in any ASCII case; the node kind agrees.
    #[test]
    fn node_kind_round_trips_in_any_case(
        kind in select(vec![NodeKind::Board, NodeKind::Folder]),
        upper in vec(any::<bool>(), 6),
    ) {
        let text: String = kind
            .as_str()
            .chars()
            .zip(upper.iter().cycle())
            .map(|(c, &up)| [c, c.to_ascii_uppercase()]
                .get(usize::from(up))
                .copied()
                .unwrap_or(c))
            .collect();
        prop_assert_eq!(NodeKind::creatable(&text)?, kind);
        prop_assert_eq!(NodeKind::from(kind.as_str()), kind);
    }

    /// The trimmed request, or the source with a suffix — never empty.
    #[test]
    fn copy_name_is_never_empty(source in board_name(), requested in option::of(board_name())) {
        let name = copy_name(&source, requested.as_deref());
        prop_assert!(!name.is_empty());
        let wanted = requested.as_deref().map(str::trim).filter(|r| !r.is_empty());
        prop_assert!(wanted.is_none_or(|wanted| name == wanted));
    }

    /// A function of its inputs naming the id, the parent and the workspace.
    #[test]
    fn new_channel_body_is_deterministic(
        name in board_name(),
        workspace in uuid(),
        folder in option::of(uuid()),
        fresh in "[a-z0-9-]{1,12}",
        ms in 0..=LAST_MS,
    ) {
        prop_assume!(!name.trim().is_empty());
        let new = NewNode {
            name: name.parse()?,
            kind: NodeKind::Board,
            workspace: workspace.parse::<WorkspaceId>()?,
            parent: folder
                .as_deref()
                .map(str::parse::<FolderId>)
                .transpose()?
                .map_or(NewParent::Root, NewParent::Folder),
        };
        let stamp = CreationStamp::at(fresh.clone(), Timestamp::from_millis(ms));
        let make = || -> crate::Result<Value> {
            Ok(serde_json::to_value(CreateChannel::new(&new, "me", &stamp, None)?)?)
        };
        let body = make()?;
        prop_assert_eq!(&body, &make()?);
        let parent = folder.as_ref().unwrap_or(&workspace).to_ascii_lowercase();
        prop_assert_eq!(body.get("id"), Some(&json!(fresh)));
        prop_assert_eq!(body.get("name"), Some(&json!(name.trim())));
        prop_assert_eq!(body.get("workspace_id"), Some(&json!(workspace.to_ascii_lowercase())));
        prop_assert_eq!(body.get("parent_id"), Some(&json!(parent)));
        prop_assert_eq!(NodeBrief::of(&body).ok().and_then(|summary| summary.parent()).map(|found| found.id().to_owned()), Some(parent));
    }

    /// An object without `id` and `name` goes out as it came.
    #[test]
    fn template_keeps_the_fields_it_does_not_name(value in json_value()) {
        let map: Map<String, Value> = value
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| !matches!(key.as_str(), "id" | "name"))
            .map(|(key, field)| (key.clone(), field.clone()))
            .collect();
        let template = Template::try_from(map.clone());
        prop_assert_eq!(
            template.ok().and_then(|template| serde_json::to_value(template).ok()),
            Some(Value::Object(map))
        );
    }

    /// Never JSON; binary bytes come back unchanged.
    #[test]
    fn backup_rejects_json_and_keeps_binary(
        value in json_value(),
        binary in vec(any::<u8>(), 0..64).prop_map(|bytes| [&[0x1f_u8, 0x8b][..], &bytes].concat()),
    ) {
        let looks_like_json = matches!(value, Value::Object(_) | Value::Array(_) | Value::String(_));
        prop_assert_eq!(Backup::try_from(value.to_string().into_bytes()).is_err(), looks_like_json);
        prop_assert_eq!(Backup::try_from(binary.clone()).map(Vec::from).ok(), Some(binary));
    }

    /// The bytes sit untouched before the boundary; the name cannot break the header.
    #[test]
    fn multipart_body_wraps_the_bytes(
        name in ".{0,12}",
        bytes in vec(any::<u8>(), 0..64),
    ) {
        let body = multipart_body("BND", "file", &name, "image/png", &bytes);
        let tail = b"\r\n--BND--\r\n";
        let header_end = body.windows(4).position(|window| window == b"\r\n\r\n").map(|at| at + 4);
        prop_assert_eq!(
            header_end.and_then(|start| body.get(start..body.len().checked_sub(tail.len())?)),
            Some(bytes.as_slice())
        );
        let header = header_end.and_then(|end| body.get(..end)).unwrap_or_default();
        prop_assert_eq!(header.windows(2).filter(|pair| pair == b"\r\n").count(), 4);
    }

    #[test]
    fn email_round_trips(
        user in "[a-z0-9]{1,10}(\\+[a-z]{1,4})?",
        domain in "[a-z]{1,8}(\\.[a-z]{1,8})?\\.[a-z]{2,4}",
        pad in "[ \t]{0,2}",
    ) {
        let raw = format!("{user}@{domain}");
        let email: Email = format!("{pad}{raw}{pad}").parse()?;
        prop_assert_eq!(email.as_str(), raw.as_str());
        prop_assert_eq!(email.to_string().parse::<Email>()?, email);
    }

    /// Without one `@` and a dotted domain it is not an address.
    #[test]
    fn email_without_at_or_dot_is_refused(raw in "[a-z0-9 .]{0,16}(@[a-z]{0,8})?") {
        let dotted_domain = raw
            .split_once('@')
            .is_some_and(|(user, domain)| !user.trim().is_empty() && domain.contains('.'));
        prop_assert!(dotted_domain || raw.parse::<Email>().is_err());
    }
}

fn chosen(
    result: crate::Result<Workspace>,
) -> Result<String, std::mem::Discriminant<crate::Error>> {
    result
        .map(|workspace| workspace.id)
        .map_err(|error| std::mem::discriminant(&error))
}

fn workspaces_with_spec()
-> impl Strategy<Value = (Vec<Workspace>, Vec<Workspace>, Option<String>)> {
    btree_map("w[0-9]{1,3}", "[a-zа-я ]{0,8}", 0..6).prop_flat_map(|map| {
        let all: Vec<_> = map
            .into_iter()
            .map(|(id, name)| Workspace { id, name })
            .collect();
        let ids: Vec<_> = all.iter().map(|workspace| workspace.id.clone()).collect();
        let spec = match ids.as_slice() {
            [] => option::of("[a-zа-я ]{0,4}").boxed(),
            _ => option::of(prop_oneof!["[a-zа-я ]{0,4}", select(ids)]).boxed(),
        };
        (Just(all.clone()), Just(all).prop_shuffle(), spec)
    })
}

proptest! {
    #![proptest_config(config())]

    /// The choice does not depend on the order of the list.
    #[test]
    fn workspace_choice_ignores_order(
        (all, shuffled, spec) in workspaces_with_spec(),
    ) {
        prop_assert_eq!(
            chosen(crate::domain::lookup::choice::workspace(&all, spec.as_deref())),
            chosen(crate::domain::lookup::choice::workspace(&shuffled, spec.as_deref()))
        );
    }

    /// Workspaces from `current_user`: no repeats, none lost.
    #[test]
    fn workspaces_are_deduplicated_and_complete(
        listed in vec(("w[0-9]{1}", option::of("[a-z]{0,4}")), 0..6),
        joined_ids in vec("w[0-9]{1}", 0..6),
    ) {
        let me = json!({
            "workspaces": listed.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "workspace_users": joined_ids.iter().map(|id| json!({ "workspace_id": id })).collect::<Vec<_>>(),
        });
        let found = account_workspaces(&me);
        let ids: Vec<_> = found.iter().map(|workspace| workspace.id.as_str()).collect();
        let unique: BTreeSet<_> = ids.iter().copied().collect();
        let all: BTreeSet<_> = listed
            .iter()
            .map(|(id, _)| id.as_str())
            .chain(joined_ids.iter().map(String::as_str))
            .collect();
        prop_assert_eq!(ids.len(), unique.len());
        prop_assert_eq!(unique, all);
    }
}
