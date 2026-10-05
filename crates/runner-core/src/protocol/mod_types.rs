use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPage<T> {
    pub items: Vec<T>,
    pub total_count: i64,
    pub filtered_count: i64,
}
