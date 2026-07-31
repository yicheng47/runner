// Tauri command handlers exposed to the frontend.
//
// Each submodule splits into pure-SQL functions (unit-testable against an
// in-memory pool) plus thin `#[tauri::command]` wrappers that pull a
// connection from the r2d2 pool and delegate. See docs/impls/archive/0001-v0-mvp.md §C2.

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

pub mod app;
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
