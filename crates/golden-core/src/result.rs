//! Structured run results consumed by the CLI reporters (Spec 1) and TUI (Spec 2).

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunResult {
    pub collections: Vec<CollectionResult>,
    pub totals: Totals,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub requests: usize,
    pub failed_requests: usize,
    pub assertions: usize,
    pub failed_assertions: usize,
    pub total_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionResult {
    pub name: String,
    pub iterations: Vec<Iteration>,
    pub stats: Vec<RequestStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Iteration {
    pub index: u32,
    pub requests: Vec<RequestResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestResult {
    pub name: String,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub time_ms: u128,
    pub assertions: Vec<Assertion>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Assertion {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestStats {
    pub name: String,
    pub avg_ms: f64,
    pub min_ms: u128,
    pub max_ms: u128,
}
