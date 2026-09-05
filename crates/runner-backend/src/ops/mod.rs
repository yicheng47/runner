// Application operations — the bodies behind every frontend command.
//
// Each submodule splits into pure-SQL functions (unit-testable against an
// in-memory pool) plus state-level functions over `AppCore` used by the GPUI
// frontend and MCP tools.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ListPage<T> {
    pub items: Vec<T>,
    pub total_count: i64,
    pub filtered_count: i64,
}

pub(super) fn escaped_like_pattern(query: &str) -> String {
    let escaped = query
        .trim()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

pub(super) fn page_limit_offset(page: i64, page_size: i64, filtered_count: i64) -> (i64, i64) {
    let page_size = page_size.max(1);
    let page_count = ((filtered_count.saturating_sub(1) / page_size) + 1).max(1);
    let page = page.clamp(1, page_count);
    (page_size, (page - 1) * page_size)
}

pub mod crew;
pub mod mcp;
pub mod mission;
pub mod node;
pub mod project;
pub mod runner;
pub mod runtime;
pub mod session;
pub mod slot;
pub mod window;
