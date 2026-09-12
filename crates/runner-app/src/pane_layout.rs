use std::collections::BTreeMap;

use anyhow::{bail, Context as _, Result};
use runner_backend::ops::node::NodeTabUpsertInput;
use runner_backend::repo::node::{NodeRow, NodeType};
use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_DRAWER_HEIGHT: f32 = 280.;
pub const MIN_DRAWER_HEIGHT: f32 = 120.;
pub const MAX_DRAWER_HEIGHT: f32 = 600.;

// The six shapes the retired layout picker could draw. Kept only so rows
// written before the tree landed still open; nothing writes a preset.
// Wire names keep the Tauri-era spellings (`cols-2`, not serde's kebab-case
// `cols2`); the aliases keep rows written by 0.6.0/0.6.1 readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum PresetKind {
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "cols-2", alias = "cols2")]
    Cols2,
    #[serde(rename = "rows-2", alias = "rows2")]
    Rows2,
    #[serde(rename = "main-2", alias = "main2")]
    Main2,
    #[serde(rename = "cols-3", alias = "cols3")]
    Cols3,
    #[serde(rename = "rows-3", alias = "rows3")]
    Rows3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitOrientation {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneLeaf {
    pub id: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneSplit {
    pub id: String,
    pub orientation: SplitOrientation,
    pub sizes: [f32; 2],
    pub a: Box<PaneNode>,
    pub b: Box<PaneNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PaneNode {
    Leaf(PaneLeaf),
    Split(PaneSplit),
}

impl PaneNode {
    pub fn leaves(&self) -> Vec<&PaneLeaf> {
        let mut leaves = Vec::new();
        self.collect_leaves(&mut leaves);
        leaves
    }

    fn collect_leaves<'a>(&'a self, leaves: &mut Vec<&'a PaneLeaf>) {
        match self {
            Self::Leaf(leaf) => leaves.push(leaf),
            Self::Split(split) => {
                split.a.collect_leaves(leaves);
                split.b.collect_leaves(leaves);
            }
        }
    }

    // Leaves are `p<n>` and splits `s<n>`, both drawn from one counter so a
    // tab never reuses a number. Legacy split ids (`cols-2:outer`) carry no
    // number of their own and simply do not raise the mark.
    fn highest_node_number(&self) -> usize {
        match self {
            Self::Leaf(leaf) => node_number(&leaf.id),
            Self::Split(split) => node_number(&split.id)
                .max(split.a.highest_node_number())
                .max(split.b.highest_node_number()),
        }
    }

    fn split_leaf(
        &mut self,
        pane_id: &str,
        split_id: &str,
        new_pane_id: &str,
        orientation: SplitOrientation,
    ) -> bool {
        match self {
            Self::Leaf(existing) => {
                if existing.id != pane_id {
                    return false;
                }
                let existing = Self::Leaf(existing.clone());
                *self = split(
                    split_id,
                    orientation,
                    [50., 50.],
                    existing,
                    leaf(new_pane_id, None),
                );
                true
            }
            Self::Split(parent) => {
                parent
                    .a
                    .split_leaf(pane_id, split_id, new_pane_id, orientation)
                    || parent
                        .b
                        .split_leaf(pane_id, split_id, new_pane_id, orientation)
            }
        }
    }

    // `slots` is the only pane membership the backend understands: it nulls a
    // slot in the stored JSON when a session is archived or deleted
    // (`repo::node::remove_session_except`) and leaves the tree alone. So the
    // tree supplies the shape and `slots` supplies the sessions, in leaf order.
    fn apply_slots(&mut self, slots: &[Option<String>], next: &mut usize) {
        match self {
            Self::Leaf(leaf) => {
                leaf.session_id = slots.get(*next).cloned().flatten();
                *next += 1;
            }
            Self::Split(split) => {
                split.a.apply_slots(slots, next);
                split.b.apply_slots(slots, next);
            }
        }
    }

    fn apply_split_sizes(&mut self, sizes: &BTreeMap<String, [f32; 2]>) {
        if let Self::Split(split) = self {
            if let Some(stored) = sizes.get(&split.id).copied() {
                if valid_sizes(stored) {
                    split.sizes = stored;
                }
            }
            split.a.apply_split_sizes(sizes);
            split.b.apply_split_sizes(sizes);
        }
    }

    fn assign_session(&mut self, pane_id: &str, session_id: &str) {
        match self {
            Self::Leaf(leaf) => {
                if leaf.id == pane_id {
                    leaf.session_id = Some(session_id.to_owned());
                } else if leaf.session_id.as_deref() == Some(session_id) {
                    leaf.session_id = None;
                }
            }
            Self::Split(split) => {
                split.a.assign_session(pane_id, session_id);
                split.b.assign_session(pane_id, session_id);
            }
        }
    }

    fn remove_session(&mut self, session_id: &str) {
        match self {
            Self::Leaf(leaf) => {
                if leaf.session_id.as_deref() == Some(session_id) {
                    leaf.session_id = None;
                }
            }
            Self::Split(split) => {
                split.a.remove_session(session_id);
                split.b.remove_session(session_id);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaneLayout {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: Option<String>,
    pub position: i64,
    pub root: PaneNode,
    pub focused_pane_id: String,
    drawer: TerminalDrawer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalDrawer {
    open: bool,
    height: f32,
    shells: Vec<String>,
    active: usize,
}

impl Default for TerminalDrawer {
    fn default() -> Self {
        Self {
            open: false,
            height: DEFAULT_DRAWER_HEIGHT,
            shells: Vec::new(),
            active: 0,
        }
    }
}

impl TerminalDrawer {
    fn normalized(mut self) -> Self {
        self.height = clamp_drawer_height(self.height);
        self.active = self.active.min(self.shells.len().saturating_sub(1));
        self
    }

    pub fn open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn set_height(&mut self, height: f32) {
        self.height = clamp_drawer_height(height);
    }

    pub fn shells(&self) -> &[String] {
        &self.shells
    }

    pub fn active_shell(&self) -> Option<&str> {
        self.shells.get(self.active).map(String::as_str)
    }

    pub fn add(&mut self, session_id: String) {
        if let Some(index) = self
            .shells
            .iter()
            .position(|existing| existing == &session_id)
        {
            self.active = index;
        } else {
            self.shells.push(session_id);
            self.active = self.shells.len() - 1;
        }
        self.open = true;
    }

    pub fn remove(&mut self, session_id: &str) -> bool {
        let Some(index) = self
            .shells
            .iter()
            .position(|existing| existing == session_id)
        else {
            return false;
        };
        self.shells.remove(index);
        if self.shells.is_empty() {
            self.active = 0;
            self.open = false;
        } else if index <= self.active {
            self.active = self.active.saturating_sub(1);
        }
        true
    }

    pub fn activate(&mut self, session_id: &str) -> bool {
        let Some(index) = self
            .shells
            .iter()
            .position(|existing| existing == session_id)
        else {
            return false;
        };
        self.active = index;
        self.open = true;
        true
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionLayout {
    pub drawer: TerminalDrawer,
}

impl MissionLayout {
    pub fn from_node_row(row: &NodeRow) -> Result<Self> {
        let mut layout: Self = row
            .layout
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .with_context(|| format!("parse mission {} layout", row.id))?
            .unwrap_or_default();
        layout.drawer = layout.drawer.normalized();
        Ok(layout)
    }

    pub fn serialize(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

// `slots` and `drawer` are the backend's contract (`repo::node::StoredLayout`
// feeds the reconciler from them), so the tree is written beside them rather
// than in place of them. `preset` and `sizes` are read-only now: a row saved
// before the tree landed rebuilds through them and is rewritten on its next
// save.
#[derive(Debug, Serialize, Deserialize)]
struct PersistedLayout {
    #[serde(default)]
    tree: Option<PaneNode>,
    #[serde(default, skip_serializing)]
    preset: Option<PresetKind>,
    #[serde(default)]
    slots: Vec<Option<String>>,
    #[serde(default, skip_serializing, deserialize_with = "lenient_sizes")]
    sizes: BTreeMap<String, [f32; 2]>,
    #[serde(default)]
    drawer: TerminalDrawer,
}

// The legacy writer could persist `[null,null]` sizes (NaN from the panel
// library stringified) and tolerated any malformed size entry;
// rejecting the whole layout for a cosmetic gutter value strands the tab.
fn lenient_sizes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, [f32; 2]>, D::Error> {
    let raw = serde_json::Value::deserialize(deserializer)?;
    let Some(entries) = raw.as_object() else {
        return Ok(BTreeMap::new());
    };
    Ok(entries
        .iter()
        .filter_map(|(id, value)| {
            let [a, b] = value.as_array()?.as_slice() else {
                return None;
            };
            Some((id.clone(), [a.as_f64()? as f32, b.as_f64()? as f32]))
        })
        .collect())
}

impl PaneLayout {
    pub fn from_node_row(row: &NodeRow) -> Result<Self> {
        let layout = row
            .layout
            .as_deref()
            .with_context(|| format!("tab node {} has no layout", row.id))?;
        let persisted: PersistedLayout =
            serde_json::from_str(layout).with_context(|| format!("parse tab {} layout", row.id))?;
        let root = match persisted.tree {
            Some(mut tree) => {
                tree.apply_slots(&persisted.slots, &mut 0);
                tree
            }
            None => {
                let preset = persisted
                    .preset
                    .context("tab layout has neither a tree nor a preset")?;
                let mut root = build_preset_tree(preset, &persisted.slots);
                root.apply_split_sizes(&persisted.sizes);
                root
            }
        };
        let leaves = root.leaves();
        let focused_pane_id = leaves
            .first()
            .context("tab layout has no panes")?
            .id
            .clone();
        let grouped = leaves.len() > 1;
        Ok(Self {
            id: row.id.clone(),
            parent_id: row.parent_id.clone(),
            name: row
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty() && grouped)
                .map(str::to_owned),
            position: row.position,
            root,
            focused_pane_id,
            drawer: persisted.drawer.normalized(),
        })
    }

    pub fn single(focused_session_id: Option<&str>, visible: &[String]) -> Self {
        let session_id = focused_session_id
            .map(str::to_owned)
            .or_else(|| visible.first().cloned());
        Self {
            id: String::new(),
            parent_id: None,
            name: None,
            position: 0,
            root: leaf("p1", session_id),
            focused_pane_id: "p1".to_owned(),
            drawer: TerminalDrawer::default(),
        }
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.root
            .leaves()
            .into_iter()
            .filter_map(|leaf| leaf.session_id.clone())
            .collect()
    }

    pub fn contains_session(&self, session_id: &str) -> bool {
        self.root
            .leaves()
            .into_iter()
            .any(|leaf| leaf.session_id.as_deref() == Some(session_id))
    }

    pub fn all_session_ids(&self) -> Vec<String> {
        self.session_ids()
            .into_iter()
            .chain(self.drawer.shells.iter().cloned())
            .collect()
    }

    pub fn drawer_open(&self) -> bool {
        self.drawer.open()
    }

    pub fn set_drawer_open(&mut self, open: bool) {
        self.drawer.set_open(open);
    }

    pub fn drawer_height(&self) -> f32 {
        self.drawer.height()
    }

    pub fn set_drawer_height(&mut self, height: f32) {
        self.drawer.set_height(height);
    }

    pub fn drawer_shells(&self) -> &[String] {
        self.drawer.shells()
    }

    pub fn active_drawer_shell(&self) -> Option<&str> {
        self.drawer.active_shell()
    }

    pub fn add_drawer_shell(&mut self, session_id: String) {
        self.drawer.add(session_id);
    }

    pub fn remove_drawer_shell(&mut self, session_id: &str) -> bool {
        self.drawer.remove(session_id)
    }

    pub fn activate_drawer_shell(&mut self, session_id: &str) -> bool {
        self.drawer.activate(session_id)
    }

    pub fn is_resize_owner(&self, pane_id: &str, session_id: &str) -> bool {
        self.root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.as_deref() == Some(session_id))
            .is_some_and(|leaf| leaf.id == pane_id)
    }

    pub fn focused_session_id(&self) -> Option<&str> {
        self.root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.id == self.focused_pane_id)
            .and_then(|leaf| leaf.session_id.as_deref())
    }

    pub fn focus_pane(&mut self, pane_id: &str) -> bool {
        if !self.root.leaves().iter().any(|leaf| leaf.id == pane_id) {
            return false;
        }
        self.focused_pane_id = pane_id.to_owned();
        true
    }

    pub fn focus_session(&mut self, session_id: &str) -> bool {
        let Some(pane_id) = self
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.as_deref() == Some(session_id))
            .map(|leaf| leaf.id.clone())
        else {
            return false;
        };
        self.focused_pane_id = pane_id;
        true
    }

    pub fn assign_session(&mut self, pane_id: &str, session_id: &str) -> Result<()> {
        if !self.root.leaves().iter().any(|leaf| leaf.id == pane_id) {
            bail!("pane not found: {pane_id}");
        }
        self.root.assign_session(pane_id, session_id);
        self.focused_pane_id = pane_id.to_owned();
        Ok(())
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.root.remove_session(session_id);
        self.remove_drawer_shell(session_id);
    }

    /// Turns `pane_id`'s leaf into a 50/50 split with a new empty pane on the
    /// given side, focuses the new pane and returns its id.
    pub fn split(&mut self, pane_id: &str, orientation: SplitOrientation) -> Result<String> {
        let next = self.root.highest_node_number() + 1;
        let split_id = format!("s{next}");
        let new_pane_id = format!("p{}", next + 1);
        if !self
            .root
            .split_leaf(pane_id, &split_id, &new_pane_id, orientation)
        {
            bail!("pane not found: {pane_id}");
        }
        self.focused_pane_id = new_pane_id.clone();
        Ok(new_pane_id)
    }

    pub fn prepare_new_pane(&mut self) -> Result<String> {
        if let Some(pane_id) = self
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.id == self.focused_pane_id && leaf.session_id.is_none())
            .map(|leaf| leaf.id.clone())
        {
            return Ok(pane_id);
        }
        if let Some(pane_id) = self
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.is_none())
            .map(|leaf| leaf.id.clone())
        {
            self.focused_pane_id = pane_id.clone();
            return Ok(pane_id);
        }

        let focused = self.focused_pane_id.clone();
        self.split(&focused, SplitOrientation::Row)
    }

    pub fn close_pane(&mut self, pane_id: &str) -> bool {
        if matches!(self.root, PaneNode::Leaf(_)) {
            return false;
        }

        let Some(root) = remove_pane(&self.root, pane_id) else {
            return false;
        };
        if root == self.root {
            return false;
        }

        let focused_pane_id = root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.id == self.focused_pane_id)
            .or_else(|| root.leaves().into_iter().next())
            .expect("collapsed pane tree has a leaf")
            .id
            .clone();
        self.root = root;
        self.focused_pane_id = focused_pane_id;
        if matches!(self.root, PaneNode::Leaf(_)) {
            self.name = None;
        }
        true
    }

    pub fn set_split_sizes(&mut self, split_id: &str, sizes: [f32; 2]) -> bool {
        if !valid_sizes(sizes) {
            return false;
        }
        fn set(node: &mut PaneNode, split_id: &str, sizes: [f32; 2]) -> bool {
            let PaneNode::Split(split) = node else {
                return false;
            };
            if split.id == split_id {
                split.sizes = sizes;
                return true;
            }
            set(&mut split.a, split_id, sizes) || set(&mut split.b, split_id, sizes)
        }
        set(&mut self.root, split_id, sizes)
    }

    pub fn serialize(&self) -> Result<String> {
        Ok(serde_json::to_string(&PersistedLayout {
            tree: Some(self.root.clone()),
            preset: None,
            slots: self
                .root
                .leaves()
                .into_iter()
                .map(|leaf| leaf.session_id.clone())
                .collect(),
            sizes: BTreeMap::new(),
            drawer: self.drawer.clone(),
        })?)
    }

    pub fn upsert_input(&self) -> Result<NodeTabUpsertInput> {
        Ok(NodeTabUpsertInput {
            id: self.id.clone(),
            parent_id: self.parent_id.clone(),
            name: self.name.clone().unwrap_or_default(),
            layout: self.serialize()?,
        })
    }
}

fn clamp_drawer_height(height: f32) -> f32 {
    if height.is_finite() {
        height.clamp(MIN_DRAWER_HEIGHT, MAX_DRAWER_HEIGHT)
    } else {
        DEFAULT_DRAWER_HEIGHT
    }
}

#[derive(Debug, Default)]
pub struct TabSet {
    tabs: Vec<PaneLayout>,
    active_tab_id: Option<String>,
}

impl TabSet {
    pub fn from_rows(rows: &[NodeRow]) -> Self {
        let tabs = rows
            .iter()
            .filter(|row| row.node_type == NodeType::Tab)
            .filter_map(|row| match PaneLayout::from_node_row(row) {
                Ok(tab) => Some(tab),
                Err(error) => {
                    tracing::warn!(tab_id = %row.id, "skipping unreadable tab layout: {error:#}");
                    None
                }
            })
            .collect::<Vec<_>>();
        let active_tab_id = tabs.first().map(|tab| tab.id.clone());
        Self {
            tabs,
            active_tab_id,
        }
    }

    pub fn tabs(&self) -> &[PaneLayout] {
        &self.tabs
    }

    pub fn active_tab_id(&self) -> Option<&str> {
        self.active_tab_id.as_deref()
    }

    pub fn active(&self) -> Option<&PaneLayout> {
        let active = self.active_tab_id.as_deref()?;
        self.tabs.iter().find(|tab| tab.id == active)
    }

    pub fn active_mut(&mut self) -> Option<&mut PaneLayout> {
        let active = self.active_tab_id.as_deref()?;
        self.tabs.iter_mut().find(|tab| tab.id == active)
    }

    pub fn activate(&mut self, tab_id: &str) -> bool {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return false;
        }
        self.active_tab_id = Some(tab_id.to_owned());
        true
    }

    pub fn activate_session(&mut self, session_id: &str) -> bool {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.contains_session(session_id))
        else {
            return false;
        };
        tab.focus_session(session_id);
        self.active_tab_id = Some(tab.id.clone());
        true
    }

    pub fn assign_to_active(&mut self, pane_id: &str, session_id: &str) -> Result<()> {
        let active_id = self
            .active_tab_id
            .clone()
            .context("no active tab for pane assignment")?;
        for tab in &mut self.tabs {
            if tab.id == active_id {
                tab.assign_session(pane_id, session_id)?;
            } else {
                tab.remove_session(session_id);
            }
        }
        Ok(())
    }

    pub fn replace_rows(&mut self, rows: &[NodeRow]) {
        let active_id = self.active_tab_id.clone();
        let focused_pane_id = self.active().map(|tab| tab.focused_pane_id.clone());
        let focused_session = self
            .active()
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned);
        let mut next = Self::from_rows(rows);
        if let Some(active_id) = active_id {
            next.activate(&active_id);
        }
        if let Some(focused_session) = focused_session {
            if let Some(active) = next.active_mut() {
                active.focus_session(&focused_session);
            }
        } else if let Some(focused_pane_id) = focused_pane_id {
            if let Some(active) = next.active_mut() {
                active.focus_pane(&focused_pane_id);
            }
        }
        *self = next;
    }
}

fn leaf(id: &str, session_id: Option<String>) -> PaneNode {
    PaneNode::Leaf(PaneLeaf {
        id: id.to_owned(),
        session_id,
    })
}

fn split(
    id: &str,
    orientation: SplitOrientation,
    sizes: [f32; 2],
    a: PaneNode,
    b: PaneNode,
) -> PaneNode {
    PaneNode::Split(PaneSplit {
        id: id.to_owned(),
        orientation,
        sizes,
        a: Box::new(a),
        b: Box::new(b),
    })
}

fn build_preset_tree(preset: PresetKind, slots: &[Option<String>]) -> PaneNode {
    let p1 = leaf("p1", slots.first().cloned().flatten());
    let p2 = leaf("p2", slots.get(1).cloned().flatten());
    let p3 = leaf("p3", slots.get(2).cloned().flatten());
    match preset {
        PresetKind::Single => p1,
        PresetKind::Cols2 => split("cols-2:outer", SplitOrientation::Row, [50., 50.], p1, p2),
        PresetKind::Rows2 => split("rows-2:outer", SplitOrientation::Column, [50., 50.], p1, p2),
        PresetKind::Main2 => split(
            "main-2:outer",
            SplitOrientation::Row,
            [60., 40.],
            p1,
            split("main-2:inner", SplitOrientation::Column, [50., 50.], p2, p3),
        ),
        PresetKind::Cols3 => split(
            "cols-3:outer",
            SplitOrientation::Row,
            [33.33, 66.67],
            p1,
            split("cols-3:inner", SplitOrientation::Row, [50., 50.], p2, p3),
        ),
        PresetKind::Rows3 => split(
            "rows-3:outer",
            SplitOrientation::Column,
            [33.33, 66.67],
            p1,
            split("rows-3:inner", SplitOrientation::Column, [50., 50.], p2, p3),
        ),
    }
}

fn remove_pane(node: &PaneNode, pane_id: &str) -> Option<PaneNode> {
    match node {
        PaneNode::Leaf(leaf) => (leaf.id != pane_id).then(|| node.clone()),
        PaneNode::Split(split) => {
            let a = remove_pane(&split.a, pane_id);
            let b = remove_pane(&split.b, pane_id);
            match (a, b) {
                (None, None) => None,
                (Some(node), None) | (None, Some(node)) => Some(node),
                (Some(a), Some(b)) => {
                    if &a == split.a.as_ref() && &b == split.b.as_ref() {
                        Some(node.clone())
                    } else {
                        Some(PaneNode::Split(PaneSplit {
                            id: split.id.clone(),
                            orientation: split.orientation,
                            sizes: split.sizes,
                            a: Box::new(a),
                            b: Box::new(b),
                        }))
                    }
                }
            }
        }
    }
}

fn node_number(id: &str) -> usize {
    id.strip_prefix('p')
        .or_else(|| id.strip_prefix('s'))
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

fn valid_sizes(sizes: [f32; 2]) -> bool {
    sizes
        .iter()
        .all(|size| size.is_finite() && *size > 0. && *size < 100.)
}

#[cfg(test)]
mod tests {
    use super::{PaneLayout, PaneNode, SplitOrientation};
    use runner_backend::repo::node::{NodeRow, NodeType};

    fn tab_row(name: Option<&str>, layout: &PaneLayout) -> NodeRow {
        NodeRow {
            id: "tab-1".into(),
            parent_id: None,
            position: 0,
            node_type: NodeType::Tab,
            name: name.map(str::to_owned),
            ref_id: None,
            layout: Some(layout.serialize().unwrap()),
            pinned_position: None,
            last_completed_at: None,
            last_viewed_at: None,
            created_at: "2026-09-10T00:00:00Z".into(),
        }
    }

    #[test]
    fn single_pane_tabs_take_their_name_from_the_session_not_the_node() {
        let single = PaneLayout::single(Some("shell"), &["shell".into()]);
        let loaded = PaneLayout::from_node_row(&tab_row(Some("build"), &single)).unwrap();
        assert_eq!(loaded.name, None);

        let mut grouped = PaneLayout::single(Some("shell"), &["shell".into()]);
        grouped.split("p1", SplitOrientation::Row).unwrap();
        let loaded = PaneLayout::from_node_row(&tab_row(Some("build"), &grouped)).unwrap();
        assert_eq!(loaded.name.as_deref(), Some("build"));
    }

    #[test]
    fn closing_a_pane_only_removes_it_from_the_tab_layout() {
        let mut layout = PaneLayout::single(Some("chat"), &["chat".into()]);
        let terminal_pane = layout.split("p1", SplitOrientation::Row).unwrap();
        layout.assign_session(&terminal_pane, "terminal").unwrap();

        assert!(layout.close_pane("p1"));

        assert!(matches!(layout.root, PaneNode::Leaf(_)));
        assert_eq!(layout.session_ids(), ["terminal"]);
    }

    #[test]
    fn preparing_a_new_pane_uses_an_empty_pane_then_splits_the_focused_one() {
        let mut empty = PaneLayout::single(Some("chat"), &["chat".into()]);
        let empty_id = empty.split("p1", SplitOrientation::Row).unwrap();
        assert_eq!(empty.prepare_new_pane().unwrap(), empty_id);

        let mut nonfocused_empty = PaneLayout::single(Some("chat"), &["chat".into()]);
        let empty_id = nonfocused_empty.split("p1", SplitOrientation::Row).unwrap();
        assert!(nonfocused_empty.focus_session("chat"));
        assert_eq!(nonfocused_empty.prepare_new_pane().unwrap(), empty_id);
        assert_eq!(nonfocused_empty.focused_pane_id, empty_id);

        let mut full = PaneLayout::single(Some("chat"), &["chat".into()]);
        let second = full.split("p1", SplitOrientation::Row).unwrap();
        full.assign_session(&second, "terminal").unwrap();
        assert!(full.focus_session("chat"));

        let target = full.prepare_new_pane().unwrap();
        assert_eq!(full.root.leaves().len(), 3);
        assert_eq!(full.root.leaves()[1].id, target);
        assert_eq!(full.focused_pane_id, target);
        assert_eq!(full.session_ids(), ["chat", "terminal"]);
    }

    #[test]
    fn splitting_a_root_leaf_numbers_the_split_and_the_new_pane_and_focuses_it() {
        for orientation in [SplitOrientation::Row, SplitOrientation::Column] {
            let mut layout = PaneLayout::single(Some("chat"), &["chat".into()]);

            let new_pane = layout.split("p1", orientation).unwrap();

            assert_eq!(new_pane, "p3");
            assert_eq!(layout.focused_pane_id, "p3");
            assert_eq!(layout.session_ids(), ["chat"]);
            let PaneNode::Split(split) = &layout.root else {
                panic!("the root leaf must become a split");
            };
            assert_eq!(split.id, "s2");
            assert_eq!(split.orientation, orientation);
            assert_eq!(split.sizes, [50., 50.]);
            assert_eq!(
                layout
                    .root
                    .leaves()
                    .iter()
                    .map(|leaf| leaf.id.as_str())
                    .collect::<Vec<_>>(),
                ["p1", "p3"]
            );
        }
    }

    #[test]
    fn splitting_a_nested_leaf_nests_again_and_close_pane_collapses_back() {
        let mut layout = PaneLayout::single(Some("a"), &["a".into()]);
        let second = layout.split("p1", SplitOrientation::Row).unwrap();
        layout.assign_session(&second, "b").unwrap();
        let before = layout.root.clone();

        let third = layout.split(&second, SplitOrientation::Column).unwrap();

        assert_eq!(third, "p5");
        assert_eq!(layout.focused_pane_id, "p5");
        assert_eq!(layout.session_ids(), ["a", "b"]);
        let PaneNode::Split(outer) = &layout.root else {
            panic!("the outer split must survive");
        };
        assert_eq!(outer.id, "s2");
        let PaneNode::Split(inner) = outer.b.as_ref() else {
            panic!("the nested leaf must become a split");
        };
        assert_eq!(inner.id, "s4");
        assert_eq!(inner.orientation, SplitOrientation::Column);

        assert!(layout.close_pane(&third));
        assert_eq!(layout.root, before);
    }

    #[test]
    fn splitting_an_unknown_pane_is_an_error() {
        let mut layout = PaneLayout::single(Some("chat"), &["chat".into()]);
        assert!(layout.split("p9", SplitOrientation::Row).is_err());
        assert!(matches!(layout.root, PaneNode::Leaf(_)));
    }
}
