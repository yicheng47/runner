use std::collections::BTreeMap;

use anyhow::{bail, Context as _, Result};
use runner_backend::ops::node::NodeTabUpsertInput;
use runner_backend::repo::node::{NodeRow, NodeType};
use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_DRAWER_HEIGHT: f32 = 280.;
pub const MIN_DRAWER_HEIGHT: f32 = 120.;
pub const MAX_DRAWER_HEIGHT: f32 = 600.;

// Wire names keep the Tauri-era spellings (`cols-2`, not serde's kebab-case
// `cols2`); the aliases keep rows written by 0.6.0/0.6.1 readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetKind {
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

impl PresetKind {
    pub const ALL: [Self; 6] = [
        Self::Single,
        Self::Cols2,
        Self::Rows2,
        Self::Main2,
        Self::Cols3,
        Self::Rows3,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "Single pane",
            Self::Cols2 => "2 side by side",
            Self::Rows2 => "2 stacked",
            Self::Main2 => "1 big + 2 stacked",
            Self::Cols3 => "3 columns",
            Self::Rows3 => "3 rows",
        }
    }

    fn pane_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Cols2 | Self::Rows2 => 2,
            Self::Main2 | Self::Cols3 | Self::Rows3 => 3,
        }
    }

    fn split_id_prefix(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Cols2 => "cols-2",
            Self::Rows2 => "rows-2",
            Self::Main2 => "main-2",
            Self::Cols3 => "cols-3",
            Self::Rows3 => "rows-3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitOrientation {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaneLeaf {
    pub id: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaneSplit {
    pub id: String,
    pub orientation: SplitOrientation,
    pub sizes: [f32; 2],
    pub a: Box<PaneNode>,
    pub b: Box<PaneNode>,
}

#[derive(Debug, Clone, PartialEq)]
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

    fn collect_split_sizes(&self, sizes: &mut BTreeMap<String, [f32; 2]>) {
        if let Self::Split(split) = self {
            sizes.insert(split.id.clone(), split.sizes);
            split.a.collect_split_sizes(sizes);
            split.b.collect_split_sizes(sizes);
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
    pub preset: PresetKind,
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

#[derive(Debug, Serialize, Deserialize)]
struct PersistedLayout {
    preset: PresetKind,
    #[serde(default)]
    slots: Vec<Option<String>>,
    #[serde(default, deserialize_with = "lenient_sizes")]
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
        let mut root = build_preset_tree(persisted.preset, &persisted.slots);
        root.apply_split_sizes(&persisted.sizes);
        let focused_pane_id = root
            .leaves()
            .first()
            .context("preset has no panes")?
            .id
            .clone();
        Ok(Self {
            id: row.id.clone(),
            parent_id: row.parent_id.clone(),
            name: row
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            position: row.position,
            preset: persisted.preset,
            root,
            focused_pane_id,
            drawer: persisted.drawer.normalized(),
        })
    }

    pub fn fresh(preset: PresetKind, focused_session_id: Option<&str>, visible: &[String]) -> Self {
        let ordered = focused_session_id
            .into_iter()
            .map(str::to_owned)
            .chain(
                visible
                    .iter()
                    .filter(|session_id| Some(session_id.as_str()) != focused_session_id)
                    .cloned(),
            )
            .take(preset.pane_count())
            .map(Some)
            .collect::<Vec<_>>();
        let root = build_preset_tree(preset, &ordered);
        let leaves = root.leaves();
        let focused_pane_id = leaves
            .iter()
            .find(|leaf| leaf.session_id.is_none())
            .or_else(|| {
                focused_session_id.and_then(|focused| {
                    leaves
                        .iter()
                        .find(|leaf| leaf.session_id.as_deref() == Some(focused))
                })
            })
            .unwrap_or(&leaves[0])
            .id
            .clone();
        Self {
            id: String::new(),
            parent_id: None,
            name: None,
            position: 0,
            preset,
            root,
            focused_pane_id,
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

    pub fn apply_preset(&mut self, preset: PresetKind) {
        let visible = self.session_ids();
        let focused = self.focused_session_id().map(str::to_owned);
        let next = Self::fresh(preset, focused.as_deref(), &visible);
        self.preset = next.preset;
        self.root = next.root;
        self.focused_pane_id = next.focused_pane_id;
        if preset == PresetKind::Single {
            self.name = None;
        }
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

        let next_preset = match self.preset {
            PresetKind::Single => PresetKind::Cols2,
            PresetKind::Cols2 => PresetKind::Cols3,
            PresetKind::Rows2 => PresetKind::Rows3,
            PresetKind::Main2 | PresetKind::Cols3 | PresetKind::Rows3 => {
                bail!("this tab already has three panes")
            }
        };
        let leaves = self.root.leaves();
        let focused = leaves
            .iter()
            .position(|leaf| leaf.id == self.focused_pane_id)
            .context("focused pane is missing")?;
        let mut slots = leaves
            .into_iter()
            .map(|leaf| leaf.session_id.clone())
            .collect::<Vec<_>>();
        slots.insert(focused + 1, None);
        self.preset = next_preset;
        self.root = build_preset_tree(next_preset, &slots);
        self.focused_pane_id = self.root.leaves()[focused + 1].id.clone();
        Ok(self.focused_pane_id.clone())
    }

    pub fn next_split_preset(&self, orientation: SplitOrientation) -> Result<PresetKind> {
        match (self.root.leaves().len(), orientation) {
            (1, SplitOrientation::Row) => Ok(PresetKind::Cols2),
            (1, SplitOrientation::Column) => Ok(PresetKind::Rows2),
            (2, SplitOrientation::Row) => Ok(PresetKind::Cols3),
            (2, SplitOrientation::Column) => Ok(PresetKind::Rows3),
            _ => bail!("this tab already has three panes"),
        }
    }

    pub fn close_pane(&mut self, pane_id: &str) -> bool {
        if matches!(self.root, PaneNode::Leaf(_)) {
            return false;
        }

        let Some(mut root) = remove_pane(&self.root, pane_id) else {
            return false;
        };
        if root == self.root {
            return false;
        }

        let preset = derive_preset(&root);
        canonicalize_split_ids(&mut root, preset);
        let focused_pane_id = root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.id == self.focused_pane_id)
            .or_else(|| root.leaves().into_iter().next())
            .expect("collapsed pane tree has a leaf")
            .id
            .clone();
        self.preset = preset;
        self.root = root;
        self.focused_pane_id = focused_pane_id;
        if self.preset == PresetKind::Single {
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
        let mut sizes = BTreeMap::new();
        self.root.collect_split_sizes(&mut sizes);
        Ok(serde_json::to_string(&PersistedLayout {
            preset: self.preset,
            slots: self
                .root
                .leaves()
                .into_iter()
                .map(|leaf| leaf.session_id.clone())
                .collect(),
            sizes,
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

fn derive_preset(root: &PaneNode) -> PresetKind {
    let PaneNode::Split(split) = root else {
        return PresetKind::Single;
    };
    match (&*split.a, &*split.b) {
        (PaneNode::Leaf(_), PaneNode::Leaf(_)) => match split.orientation {
            SplitOrientation::Row => PresetKind::Cols2,
            SplitOrientation::Column => PresetKind::Rows2,
        },
        (_, PaneNode::Split(inner)) => match (split.orientation, inner.orientation) {
            (SplitOrientation::Row, SplitOrientation::Column) => PresetKind::Main2,
            (SplitOrientation::Row, SplitOrientation::Row) => PresetKind::Cols3,
            (SplitOrientation::Column, _) => PresetKind::Rows3,
        },
        (PaneNode::Split(_), PaneNode::Leaf(_)) => match split.orientation {
            SplitOrientation::Row => PresetKind::Cols3,
            SplitOrientation::Column => PresetKind::Rows3,
        },
    }
}

fn canonicalize_split_ids(root: &mut PaneNode, preset: PresetKind) {
    let PaneNode::Split(outer) = root else {
        return;
    };
    outer.id = format!("{}:outer", preset.split_id_prefix());
    if let PaneNode::Split(inner) = outer.a.as_mut() {
        inner.id = format!("{}:inner", preset.split_id_prefix());
    }
    if let PaneNode::Split(inner) = outer.b.as_mut() {
        inner.id = format!("{}:inner", preset.split_id_prefix());
    }
}

fn valid_sizes(sizes: [f32; 2]) -> bool {
    sizes
        .iter()
        .all(|size| size.is_finite() && *size > 0. && *size < 100.)
}

#[cfg(test)]
mod tests {
    use super::{PaneLayout, PresetKind, SplitOrientation};

    #[test]
    fn closing_a_pane_only_removes_it_from_the_tab_layout() {
        let visible = vec!["chat".to_owned(), "terminal".to_owned()];
        let mut layout = PaneLayout::fresh(PresetKind::Cols2, Some("chat"), &visible);
        let chat_pane = layout
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.as_deref() == Some("chat"))
            .unwrap()
            .id
            .clone();

        assert!(layout.close_pane(&chat_pane));

        assert_eq!(layout.preset, PresetKind::Single);
        assert_eq!(layout.session_ids(), ["terminal"]);
    }

    #[test]
    fn preparing_a_new_pane_uses_focus_then_splits_up_to_three() {
        let mut empty = PaneLayout::fresh(PresetKind::Cols2, Some("chat"), &["chat".into()]);
        let empty_id = empty.focused_pane_id.clone();
        assert_eq!(empty.prepare_new_pane().unwrap(), empty_id);
        assert_eq!(empty.preset, PresetKind::Cols2);

        let mut nonfocused_empty =
            PaneLayout::fresh(PresetKind::Cols2, Some("chat"), &["chat".into()]);
        let empty_id = nonfocused_empty.focused_pane_id.clone();
        assert!(nonfocused_empty.focus_session("chat"));
        assert_eq!(nonfocused_empty.prepare_new_pane().unwrap(), empty_id);
        assert_eq!(nonfocused_empty.focused_pane_id, empty_id);
        assert_eq!(nonfocused_empty.preset, PresetKind::Cols2);

        let mut capped_with_empty = PaneLayout::fresh(
            PresetKind::Cols3,
            Some("chat"),
            &["chat".into(), "terminal".into()],
        );
        let empty_id = capped_with_empty.focused_pane_id.clone();
        assert!(capped_with_empty.focus_session("chat"));
        assert_eq!(capped_with_empty.prepare_new_pane().unwrap(), empty_id);
        assert_eq!(capped_with_empty.focused_pane_id, empty_id);
        assert_eq!(capped_with_empty.preset, PresetKind::Cols3);

        let mut single = PaneLayout::fresh(PresetKind::Single, Some("chat"), &["chat".into()]);
        let target = single.prepare_new_pane().unwrap();
        assert_eq!(single.preset, PresetKind::Cols2);
        assert_eq!(single.root.leaves()[1].id, target);
        assert_eq!(single.session_ids(), ["chat"]);

        single.assign_session(&target, "terminal").unwrap();
        let target = single.prepare_new_pane().unwrap();
        assert_eq!(single.preset, PresetKind::Cols3);
        assert_eq!(single.root.leaves()[2].id, target);
        single.assign_session(&target, "second-terminal").unwrap();
        assert!(single.prepare_new_pane().is_err());
    }

    #[test]
    fn applying_two_pane_presets_leaves_the_new_pane_empty_and_focused() {
        for preset in [PresetKind::Cols2, PresetKind::Rows2] {
            let mut layout = PaneLayout::fresh(PresetKind::Single, Some("chat"), &["chat".into()]);

            layout.apply_preset(preset);

            assert_eq!(layout.session_ids(), ["chat"]);
            assert_eq!(layout.root.leaves().len(), 2);
            assert!(layout.focused_session_id().is_none());
        }
    }

    #[test]
    fn split_shortcuts_grow_rows_or_columns_and_stop_at_three() {
        for (orientation, two, three) in [
            (SplitOrientation::Row, PresetKind::Cols2, PresetKind::Cols3),
            (
                SplitOrientation::Column,
                PresetKind::Rows2,
                PresetKind::Rows3,
            ),
        ] {
            let mut layout = PaneLayout::fresh(PresetKind::Single, Some("chat"), &["chat".into()]);
            assert_eq!(layout.next_split_preset(orientation).unwrap(), two);
            layout.apply_preset(two);
            assert_eq!(layout.next_split_preset(orientation).unwrap(), three);
            layout.apply_preset(three);
            assert!(layout.next_split_preset(orientation).is_err());
        }
    }
}
