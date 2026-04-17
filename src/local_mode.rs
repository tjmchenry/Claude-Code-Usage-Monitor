use std::time::{Duration, UNIX_EPOCH};

use crate::models::{DataSource, RateLimitBucket, RateLimits, UsageData, UsageSection};
use crate::poller::{PollError, UsageSource};

/// Reads the last rate_limits snapshot that the statusLine hook forwarded via
/// WM_COPYDATA. No I/O, no network — just copies what's already in memory.
/// Returns `PollError::NoData` when no heartbeat has ever arrived, so Hybrid
/// can fall back to ApiSource on a brand-new session.
pub struct LocalSource<'a> {
    pub rate_limits: Option<&'a RateLimits>,
}

impl UsageSource for LocalSource<'_> {
    fn poll(&self) -> Result<UsageData, PollError> {
        let Some(rl) = self.rate_limits else {
            return Err(PollError::NoData);
        };
        to_usage_data(rl, DataSource::Local).ok_or(PollError::NoData)
    }
}

fn to_usage_data(rl: &RateLimits, source: DataSource) -> Option<UsageData> {
    let session = rl.five_hour.as_ref().map(bucket_to_section);
    let weekly = rl.seven_day.as_ref().map(bucket_to_section);
    // Require at least one bucket so the widget always has something to render.
    if session.is_none() && weekly.is_none() {
        return None;
    }
    Some(UsageData {
        session: session.unwrap_or_default(),
        weekly: weekly.unwrap_or_default(),
        source,
    })
}

fn bucket_to_section(b: &RateLimitBucket) -> UsageSection {
    let resets_at = if b.resets_at_unix > 0 {
        Some(UNIX_EPOCH + Duration::from_secs(b.resets_at_unix as u64))
    } else {
        None
    };
    UsageSection {
        percentage: b.used,
        resets_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(used: f64, resets: i64) -> RateLimitBucket {
        RateLimitBucket {
            used,
            resets_at_unix: resets,
        }
    }

    #[test]
    fn local_source_no_data_when_nothing_cached() {
        let src = LocalSource { rate_limits: None };
        assert!(matches!(src.poll(), Err(PollError::NoData)));
    }

    #[test]
    fn local_source_returns_both_buckets() {
        let rl = RateLimits {
            five_hour: Some(bucket(23.5, 1_900_000_000)),
            seven_day: Some(bucket(41.2, 1_900_100_000)),
        };
        let src = LocalSource {
            rate_limits: Some(&rl),
        };
        let data = src.poll().expect("should produce data");
        assert_eq!(data.source, DataSource::Local);
        assert_eq!(data.session.percentage, 23.5);
        assert_eq!(data.weekly.percentage, 41.2);
        assert!(data.session.resets_at.is_some());
        assert!(data.weekly.resets_at.is_some());
    }

    #[test]
    fn local_source_allows_partial_buckets() {
        let rl = RateLimits {
            five_hour: Some(bucket(10.0, 0)),
            seven_day: None,
        };
        let src = LocalSource {
            rate_limits: Some(&rl),
        };
        let data = src.poll().expect("should produce data");
        assert_eq!(data.session.percentage, 10.0);
        assert!(data.session.resets_at.is_none()); // resets_at_unix == 0 is "unknown"
        assert_eq!(data.weekly.percentage, 0.0); // default when missing
    }

    #[test]
    fn local_source_errors_on_empty_buckets() {
        let rl = RateLimits {
            five_hour: None,
            seven_day: None,
        };
        let src = LocalSource {
            rate_limits: Some(&rl),
        };
        assert!(matches!(src.poll(), Err(PollError::NoData)));
    }
}
