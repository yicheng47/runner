use std::collections::BTreeMap;

use anyhow::{bail, Context as _, Result};
use runner_app::ops::node::NodeTabUpsertInput;
use runner_app::repo::node::{NodeRow, NodeType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresetKind {
    Single,
    Cols2,
    Rows2,
    Main2,
    Cols3,
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
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedLayout {
    preset: PresetKind,
    #[serde(default)]
    slots: Vec<Option<String>>,
    #[serde(default)]
    sizes: BTreeMap<String, [f32; 2]>,
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

#[derive(Debug, Default)]
pub struct TabSet {
    tabs: Vec<PaneLayout>,
    active_tab_id: Option<String>,
}

impl TabSet {
    pub fn from_rows(rows: &[NodeRow]) -> Result<Self> {
        let tabs = rows
            .iter()
            .filter(|row| row.node_type == NodeType::Tab)
            .map(PaneLayout::from_node_row)
            .collect::<Result<Vec<_>>>()?;
        let active_tab_id = tabs.first().map(|tab| tab.id.clone());
        Ok(Self {
            tabs,
            active_tab_id,
        })
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

    pub fn replace_rows(&mut self, rows: &[NodeRow]) -> Result<()> {
        let active_id = self.active_tab_id.clone();
        let focused_pane_id = self.active().map(|tab| tab.focused_pane_id.clone());
        let focused_session = self
            .active()
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned);
        let mut next = Self::from_rows(rows)?;
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
        Ok(())
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

fn valid_sizes(sizes: [f32; 2]) -> bool {
    sizes
        .iter()
        .all(|size| size.is_finite() && *size > 0. && *size < 100.)
}
