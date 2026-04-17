use std::io::Read;
use std::path::PathBuf;

use serde::Deserialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, SendMessageW, WM_COPYDATA};

use crate::models::{RateLimitBucket, RateLimits};
use crate::native_interop::{wide_str, COPYDATA_RATE_LIMITS, WIDGET_WINDOW_CLASS, WM_APP_HEARTBEAT};
use crate::poller;

#[derive(Deserialize)]
struct StatuslineInput {
    session_id: Option<String>,
    #[serde(default)]
    rate_limits: Option<HookRateLimits>,
}

#[derive(Deserialize)]
struct HookRateLimits {
    #[serde(default)]
    five_hour: Option<HookBucket>,
    #[serde(default)]
    seven_day: Option<HookBucket>,
}

#[derive(Deserialize)]
struct HookBucket {
    /// Claude Code has used `used_percentage` in some releases and
    /// `utilization` in others; accept either.
    #[serde(default, alias = "utilization")]
    used_percentage: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

/// Entry point invoked when the exe is launched as a Claude Code statusLine hook.
/// Reads the JSON hook payload on stdin, posts a heartbeat and (when present)
/// the `rate_limits` snapshot to the running widget, and prints the cached
/// usage line to stdout for Claude Code's own status bar. Returns an exit code.
pub fn run() -> i32 {
    let payload = read_stdin_to_string();
    let input: Option<StatuslineInput> = serde_json::from_str(&payload).ok();
    let (session_id, rate_limits) = match input {
        Some(i) => (
            i.session_id.unwrap_or_default(),
            i.rate_limits.and_then(convert_rate_limits),
        ),
        None => (String::new(), None),
    };

    let hwnd = find_widget_window();

    if !session_id.is_empty() {
        if let Some(h) = hwnd {
            send_heartbeat(h, &session_id);
        }
    }

    if let (Some(h), Some(ref rl)) = (hwnd, rate_limits.as_ref()) {
        send_rate_limits(h, rl);
    }

    print_status_line(rate_limits.as_ref());

    0
}

fn read_stdin_to_string() -> String {
    let mut buffer = String::new();
    let _ = std::io::stdin().read_to_string(&mut buffer);
    buffer
}

fn find_widget_window() -> Option<HWND> {
    let class = wide_str(WIDGET_WINDOW_CLASS);
    unsafe {
        match FindWindowW(PCWSTR::from_raw(class.as_ptr()), PCWSTR::null()) {
            Ok(h) if h != HWND::default() => Some(h),
            _ => None,
        }
    }
}

fn send_heartbeat(hwnd: HWND, session_id: &str) {
    let hash = fnv1a_64(session_id.as_bytes());
    unsafe {
        // PostMessageW is non-blocking: if the widget is busy, we move on.
        // WPARAM is usize; on 64-bit this fits the full 64-bit hash. On 32-bit
        // we truncate, which is fine — the hash is only used for bookkeeping.
        let _ = PostMessageW(hwnd, WM_APP_HEARTBEAT, WPARAM(hash as usize), LPARAM(0));
    }
}

fn send_rate_limits(hwnd: HWND, rl: &RateLimits) {
    let Ok(json) = serde_json::to_vec(rl) else {
        return;
    };
    let mut cds = COPYDATASTRUCT {
        dwData: COPYDATA_RATE_LIMITS,
        cbData: json.len() as u32,
        lpData: json.as_ptr() as *mut _,
    };
    unsafe {
        // SendMessageW blocks until the receiver returns, which is what we want:
        // Windows only guarantees the payload buffer's validity for that span,
        // and the helper process naturally outlives the call.
        SendMessageW(
            hwnd,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&mut cds as *mut _ as isize),
        );
    }
}

fn convert_rate_limits(raw: HookRateLimits) -> Option<RateLimits> {
    let five_hour = raw.five_hour.and_then(convert_bucket);
    let seven_day = raw.seven_day.and_then(convert_bucket);
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(RateLimits {
        five_hour,
        seven_day,
    })
}

fn convert_bucket(raw: HookBucket) -> Option<RateLimitBucket> {
    let used = raw.used_percentage?;
    let resets_at_unix = poller::iso8601_to_unix(raw.resets_at.as_deref()).unwrap_or(0);
    Some(RateLimitBucket {
        used,
        resets_at_unix,
    })
}

/// Prints the status line for Claude Code's own status bar. Prefers the
/// freshly-parsed rate_limits when available; falls back to the cached
/// `current_usage.json` written by the main widget process.
fn print_status_line(rl: Option<&RateLimits>) {
    if let Some(rl) = rl {
        if let (Some(five), Some(seven)) = (rl.five_hour.as_ref(), rl.seven_day.as_ref()) {
            println!(
                "5h: {:.0}% | 7d: {:.0}%",
                five.used.round(),
                seven.used.round()
            );
            return;
        }
    }
    print_cached_status_line();
}

fn print_cached_status_line() {
    let Some(path) = current_usage_path() else {
        return;
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    let session = parsed
        .get("session_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let weekly = parsed
        .get("weekly_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if session.is_empty() && weekly.is_empty() {
        return;
    }

    println!("5h: {session} | 7d: {weekly}");
}

fn current_usage_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("ClaudeCodeUsageMonitor")
            .join("current_usage.json"),
    )
}

/// 64-bit FNV-1a hash. Used to pack a session_id into a single WPARAM.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    #[test]
    fn fnv1a_known_values() {
        // Well-known FNV-1a 64-bit test vectors.
        assert_eq!(fnv1a_64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn fnv1a_distinct_inputs_hash_distinctly() {
        let a = fnv1a_64(b"session-abc");
        let b = fnv1a_64(b"session-xyz");
        assert_ne!(a, b);
    }

    /// Mirror of the sweep logic used by window::sweep_stale_sessions to
    /// exercise its retain rule without a Windows dependency.
    fn sweep(map: &mut HashMap<u64, Instant>, now: Instant, threshold: Duration) -> usize {
        let before = map.len();
        map.retain(|_, last| now.duration_since(*last) < threshold);
        before - map.len()
    }

    #[test]
    fn sweep_drops_stale_and_keeps_fresh() {
        let now = Instant::now();
        let mut map: HashMap<u64, Instant> = HashMap::new();
        map.insert(1, now - Duration::from_secs(10));
        map.insert(2, now - Duration::from_secs(1));
        let dropped = sweep(&mut map, now, Duration::from_secs(6));
        assert_eq!(dropped, 1);
        assert!(map.contains_key(&2));
        assert!(!map.contains_key(&1));
    }

    #[test]
    fn convert_rate_limits_drops_if_both_missing() {
        let raw = HookRateLimits {
            five_hour: None,
            seven_day: None,
        };
        assert!(convert_rate_limits(raw).is_none());
    }

    #[test]
    fn convert_rate_limits_keeps_partial() {
        let raw = HookRateLimits {
            five_hour: Some(HookBucket {
                used_percentage: Some(42.0),
                resets_at: Some("2026-04-17T12:00:00Z".to_string()),
            }),
            seven_day: None,
        };
        let out = convert_rate_limits(raw).expect("should keep five_hour");
        let fh = out.five_hour.expect("five_hour present");
        assert_eq!(fh.used, 42.0);
        assert!(fh.resets_at_unix > 0);
        assert!(out.seven_day.is_none());
    }
}
