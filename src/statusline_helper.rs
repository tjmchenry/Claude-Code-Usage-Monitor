use std::io::Read;
use std::path::PathBuf;

use serde::Deserialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

use crate::native_interop::{wide_str, WIDGET_WINDOW_CLASS, WM_APP_HEARTBEAT};

#[derive(Deserialize)]
struct StatuslineInput {
    session_id: Option<String>,
}

/// Entry point invoked when the exe is launched as a Claude Code statusLine hook.
/// Reads the JSON hook payload on stdin, posts a heartbeat to the running widget,
/// and prints the cached usage line to stdout for Claude Code's own status bar.
/// Returns a process exit code.
pub fn run() -> i32 {
    let payload = read_stdin_to_string();
    let input: Option<StatuslineInput> = serde_json::from_str(&payload).ok();
    let session_id = input.and_then(|i| i.session_id).unwrap_or_default();

    if !session_id.is_empty() {
        send_heartbeat(&session_id);
    }

    print_cached_usage_line();

    0
}

fn read_stdin_to_string() -> String {
    let mut buffer = String::new();
    let _ = std::io::stdin().read_to_string(&mut buffer);
    buffer
}

fn send_heartbeat(session_id: &str) {
    let hash = fnv1a_64(session_id.as_bytes());
    let class = wide_str(WIDGET_WINDOW_CLASS);
    unsafe {
        let hwnd = match FindWindowW(PCWSTR::from_raw(class.as_ptr()), PCWSTR::null()) {
            Ok(h) if h != HWND::default() => h,
            _ => return,
        };
        // PostMessageW is non-blocking: if the widget isn't running or is busy, we move on.
        // WPARAM is usize; on 64-bit this fits the full 64-bit hash. On 32-bit we truncate,
        // which is fine — the hash is only used for stale-session bookkeeping.
        let _ = PostMessageW(hwnd, WM_APP_HEARTBEAT, WPARAM(hash as usize), LPARAM(0));
    }
}

fn print_cached_usage_line() {
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
}
