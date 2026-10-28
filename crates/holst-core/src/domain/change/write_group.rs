//! Groups made and dissolved: the group object and its members' parents.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    iter,
};

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{Any, Map as _, MapPrelim, TransactionMut};

use crate::{
    domain::{
        area::scene::Scene,
        board::{Object, ObjectKind, any, object_of},
        change::{
            check::{kind_of, parent_of, shows_content},
            journal,
            value::{ChangedObject, ObjectChangeKind},
            write::{WriteTransaction, insert_changes, object_map},
        },
        geometry::Rectangle,
    },
    error::Result,
    protocol::document::{
        ADDITIONAL_POSITION_INFO,
        CREATED,
        ID,
        IGNORE_Z_INDEX,
        PARENT_ID,
        POSITION,
        TYPE,
        X,
        Y,
        Z_INDEX,
    },
};

/// A new group around `members`, as the web client makes one: at the corner of
/// their box, as high as the highest of them, in the frame or table they all
/// share if it takes a group; the members keep their places and name the group
/// as their parent.
pub(super) fn group(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    members: &[String],
) -> Result<Vec<ChangedObject>> {
    let WriteTransaction {
        roots,
        states,
        stamp,
        context,
    } = *scope;
    let id = context.ids.new_id();
    let layer = members
        .iter()
        .filter_map(|member| states.get(member)?.get(Z_INDEX)?.as_f64())
        .reduce(f64::max)
        .unwrap_or_default();
    let corner = corner_of(states, members);
    let own = [
        (ID, json!(id)),
        (TYPE, json!(ObjectKind::Group.as_str())),
        (Z_INDEX, json!(layer)),
        (CREATED, stamp.json()),
        ("updated", Value::Null),
        (POSITION, json!({ (X): corner.0, (Y): corner.1 })),
        (IGNORE_Z_INDEX, json!(true)),
    ];
    let pairs = own
        .into_iter()
        .chain(home_of(states, members))
        .map(|(name, value)| any(&value).map(|value| (name.to_owned(), value)))
        .collect::<Result<Vec<(String, Any)>>>()?;
    roots
        .objects
        .insert(transaction, id.as_str(), MapPrelim::from_iter(pairs));
    let moved = members
        .iter()
        .map(|member| {
            let kind = kind_of(states, member);
            let map = object_map(transaction, &roots.objects, member, &kind)?;
            insert_changes(
                transaction,
                &map,
                &[(PARENT_ID, json!(id)), ("updated", stamp.json())],
            )?;
            Ok(ChangedObject {
                id: member.clone(),
                kind,
                change: ObjectChangeKind::Changed,
                fields: vec!["parent"],
                link: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(iter::once(ChangedObject {
        id,
        kind: ObjectKind::Group,
        change: ObjectChangeKind::Created,
        fields: Vec::new(),
        link: None,
    })
    .chain(moved)
    .collect())
}

/// The corner of the box the members cover.
fn corner_of(states: &HashMap<String, Value>, members: &[String]) -> (f64, f64) {
    let objects: Vec<Object> = members
        .iter()
        .filter_map(|member| states.get(member))
        .filter_map(|fields| object_of(fields.clone()))
        .collect();
    let scene = Scene::new(&objects);
    Rectangle::bounding(objects.iter().filter_map(|object| scene.covered_by(object)))
        .map_or((0.0, 0.0), |area| (area.x, area.y))
}

/// What puts the group where its members stand: the frame or table they all
/// share, if it takes a group, with the table cell the first member sits in.
fn home_of(
    states: &HashMap<String, Value>,
    members: &[String],
) -> Vec<(&'static str, Value)> {
    let shared = members
        .iter()
        .map(|member| states.get(member).and_then(parent_of))
        .all_equal_value()
        .ok()
        .flatten()
        .filter(|parent| takes_group(states, parent));
    shared.map_or_else(Vec::new, |parent| {
        let cell = members
            .first()
            .and_then(|first| states.get(first)?.get(ADDITIONAL_POSITION_INFO))
            .filter(|info| !info.is_null())
            .map(|info| (ADDITIONAL_POSITION_INFO, info.clone()));
        iter::once((PARENT_ID, json!(parent))).chain(cell).collect()
    })
}

/// Whether the object holds a group as a child: a frame that shows its content,
/// or a table.
fn takes_group(states: &HashMap<String, Value>, parent: &str) -> bool {
    match kind_of(states, parent) {
        ObjectKind::Frame => shows_content(states, parent),
        ObjectKind::Table => true,
        ObjectKind::Sticker
        | ObjectKind::StickerStack
        | ObjectKind::Text
        | ObjectKind::Shape
        | ObjectKind::Document
        | ObjectKind::Arrow
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Stamp
        | ObjectKind::Group
        | ObjectKind::TableCell
        | ObjectKind::Code
        | ObjectKind::Drawing
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => false,
    }
}

/// Groups that go, and the parent each member that stays lands in.
#[derive(Debug, Default)]
struct Dissolution {
    groups: Vec<String>,
    moved: BTreeMap<String, Option<String>>,
}

impl Dissolution {
    /// The parent of `id` as the dissolutions so far leave it.
    fn parent(&self, states: &HashMap<String, Value>, id: &str) -> Option<String> {
        self.moved
            .get(id)
            .cloned()
            .unwrap_or_else(|| states.get(id).and_then(parent_of).map(str::to_owned))
    }

    /// What stays in `group`: not what `gone` takes, nor a group already
    /// dissolved.
    fn members(
        &self,
        states: &HashMap<String, Value>,
        group: &str,
        gone: &BTreeSet<String>,
    ) -> Vec<String> {
        states
            .keys()
            .filter(|id| !gone.contains(*id) && !self.groups.contains(id))
            .filter(|id| self.parent(states, id).as_deref() == Some(group))
            .sorted_unstable()
            .cloned()
            .collect()
    }

    /// `self` with `group` dissolved: its `members` go to its parent.
    fn dissolving(
        self,
        states: &HashMap<String, Value>,
        group: &str,
        members: Vec<String>,
    ) -> Self {
        let up = self.parent(states, group);
        Self {
            groups: self.groups.into_iter().chain([group.to_owned()]).collect(),
            moved: self
                .moved
                .into_iter()
                .chain(members.into_iter().map(|member| (member, up.clone())))
                .collect(),
        }
    }

    /// Writes it: the members re-parented, the groups removed and journalled.
    fn written(
        &self,
        transaction: &mut TransactionMut<'_>,
        scope: &WriteTransaction<'_>,
    ) -> Result<Vec<ChangedObject>> {
        let WriteTransaction {
            roots,
            states,
            stamp,
            ..
        } = *scope;
        let moved = self
            .moved
            .iter()
            .filter(|(member, _)| !self.groups.contains(*member))
            .map(|(member, up)| {
                let kind = kind_of(states, member);
                let map = object_map(transaction, &roots.objects, member, &kind)?;
                insert_changes(
                    transaction,
                    &map,
                    &[(PARENT_ID, json!(up)), ("updated", stamp.json())],
                )?;
                Ok(ChangedObject {
                    id: member.clone(),
                    kind,
                    change: ObjectChangeKind::Changed,
                    fields: vec!["parent"],
                    link: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let removed: Vec<ChangedObject> = self
            .groups
            .iter()
            .map(|group| {
                roots.objects.remove(transaction, group);
                ChangedObject {
                    id: group.clone(),
                    kind: ObjectKind::Group,
                    change: ObjectChangeKind::Removed,
                    fields: Vec::new(),
                    link: None,
                }
            })
            .collect();
        if !self.groups.is_empty() {
            let gone: Vec<&str> = self.groups.iter().map(String::as_str).collect();
            journal::write(
                transaction,
                &roots.journal,
                journal::entry(states, &gone, stamp),
            )?;
        }
        Ok(moved.into_iter().chain(removed).collect())
    }
}

/// The groups dissolved, their members left where the groups stood.
pub(super) fn ungroup(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    groups: &[String],
) -> Result<Vec<ChangedObject>> {
    let states = scope.states;
    let none = BTreeSet::new();
    groups
        .iter()
        .fold(Dissolution::default(), |done, group| {
            let members = done.members(states, group, &none);
            done.dissolving(states, group, members)
        })
        .written(transaction, scope)
}

/// The groups the removal of `gone` leaves with fewer than two members,
/// dissolved as the web client does: the member left goes to its group's
/// parent, and that parent may be left short in its turn. (Nothing is removed
/// from a locked group, so a lock never keeps a group short of members.)
pub(super) fn dissolve_after_removal(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    gone: &BTreeSet<String>,
) -> Result<Vec<ChangedObject>> {
    let states = scope.states;
    let mut queue: Vec<String> = gone
        .iter()
        .filter_map(|id| states.get(id).and_then(parent_of))
        .map(str::to_owned)
        .unique()
        .collect();
    let mut dissolution = Dissolution::default();
    // imperative: a group that goes may leave its own parent short of members
    while let Some(group) = queue.pop() {
        let candidate = !gone.contains(&group)
            && !dissolution.groups.contains(&group)
            && kind_of(states, &group) == ObjectKind::Group;
        let members = dissolution.members(states, &group, gone);
        if candidate && members.len() < 2 {
            queue.extend(dissolution.parent(states, &group));
            dissolution = dissolution.dissolving(states, &group, members);
        }
    }
    dissolution.written(transaction, scope)
}
