use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataSource {
    #[default]
    Api,
    Local,
    Hybrid,
}

#[derive(Clone, Debug, Default)]
pub struct UsageSection {
    pub percentage: f64,
    pub resets_at: Option<SystemTime>,
}

#[derive(Clone, Debug, Default)]
pub struct UsageData {
    pub session: UsageSection,
    pub weekly: UsageSection,
    pub source: DataSource,
}

/// Raw rate-limit snapshot forwarded by the statusLine hook. Percentages are
/// server-authoritative and cover the same five-hour / seven-day buckets the
/// API returns.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RateLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<RateLimitBucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<RateLimitBucket>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct RateLimitBucket {
    /// Used percentage (0.0–100.0).
    #[serde(rename = "u")]
    pub used: f64,
    /// Reset time as unix epoch seconds.
    #[serde(rename = "r")]
    pub resets_at_unix: i64,
}
