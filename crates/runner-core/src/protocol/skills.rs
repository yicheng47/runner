use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GlobalState {
    On,
    Off,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub marker: String,
    pub symlink: Option<PathBuf>,
    pub manual: bool,
    pub hidden: bool,
    pub problem: Option<String>,
    pub files: Vec<String>,
    pub global: GlobalState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCatalog {
    pub runtime: Runtime,
    pub roots: Vec<PathBuf>,
    pub root_exists: bool,
    pub entries: Vec<SkillEntry>,
}

#[derive(Default, Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SkillDocument {
    pub frontmatter: Vec<(String, String)>,
    pub body: String,
    pub problem: Option<String>,
}

impl SkillDocument {
    pub fn value(&self, key: &str) -> Option<&str> {
        self.frontmatter
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

pub fn parse_skill_document(text: &str) -> SkillDocument {
    let mut document = SkillDocument {
        body: text.to_owned(),
        ..SkillDocument::default()
    };
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next().filter(|line| line.trim_end() == "---") else {
        return document;
    };
    let mut offset = first.len();
    let mut closed = false;
    let mut block: Option<(String, bool, Vec<String>)> = None;
    for line in lines {
        offset += line.len();
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, _, values)) = block.as_mut() {
                values.push(line.trim().to_owned());
            }
            continue;
        }
        finish_block(&mut block, &mut document.frontmatter);
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line
            .split_once(':')
            .filter(|(key, _)| !key.trim().is_empty())
        else {
            document.problem = Some("malformed frontmatter".into());
            continue;
        };
        let key = key.trim();
        if !matches!(
            key,
            "name" | "description" | "disable-model-invocation" | "user-invocable"
        ) {
            continue;
        }
        let value = value.trim();
        if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            block = Some((key.into(), value.starts_with('>'), Vec::new()));
            continue;
        }
        let value = if value.starts_with('"') {
            serde_json::from_str::<String>(value).ok()
        } else if value.starts_with('\'') {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .map(|value| value.replace("''", "'"))
        } else if value.starts_with(['[', '{']) {
            None
        } else {
            Some(
                value
                    .split(" #")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            )
        };
        match value {
            Some(value)
                if !matches!(key, "disable-model-invocation" | "user-invocable")
                    || matches!(value.as_str(), "true" | "false") =>
            {
                document.frontmatter.push((key.into(), value));
            }
            _ => document.problem = Some(format!("malformed frontmatter: {key}")),
        }
    }
    finish_block(&mut block, &mut document.frontmatter);
    if closed {
        document.body = text[offset..].to_owned();
    } else {
        document.problem = Some("unclosed frontmatter".into());
    }
    document
}

fn finish_block(block: &mut Option<(String, bool, Vec<String>)>, rows: &mut Vec<(String, String)>) {
    if let Some((key, folded, values)) = block.take() {
        rows.push((key, values.join(if folded { " " } else { "\n" })));
    }
}
