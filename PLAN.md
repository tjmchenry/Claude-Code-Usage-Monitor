# Plan: Local-only mode, auto-hide, and upstream-prep

**Fork-local. Do not include in upstream PRs.** Same rule as `CLAUDE.md`. When preparing the upstream PR, either base the PR branch on `upstream/main` or drop any commits that touch this file.

## Status as of this document

Phase 1 of the prior plan shipped on commit `3267623` (amended by `27bb98f` for a clippy fix). The `statusLine` helper, heartbeat-driven auto-hide toggle, and hook installer UI are all wired up and working. Audit details and remaining gaps are below. Phase 2 (Local data source) has **not** started — zero lines of it exist in the tree.

### Phase 1 audit

| Plan section | Status | Evidence |
|---|---|---|
| 1.1 `statusline` subcommand | Done | `src/main.rs:29`, `src/statusline_helper.rs` |
| 1.2 Heartbeat handling | Done | `WM_APP_HEARTBEAT` at `src/native_interop.rs:26`, handler at `src/window.rs:1708`, sweep at `src/window.rs:401`, no `ReadDirectoryChangesW` |
| 1.3 Visibility composition | Mostly done | `compute_effective_visibility` at `src/window.rs:331`, `apply_visibility` at `src/window.rs:351`. **Gap:** no tray-icon tooltip note when `auto_hide_when_idle` is on but the hook has never heartbeated |
| 1.4 Hook install UI | Mostly done | `src/hook_installer.rs` full, menu branches at `src/window.rs:2036`. **Gap:** `OtherInstalled` flow offers Replace/Cancel only, not Chain. Chain was labeled stretch in the prior plan; carry forward as stretch |
| 1.5 Settings file | Done enough | `auto_hide_when_idle` and `stale_session_secs` persisted at `src/window.rs:240`. `statusline_hook_installed` was skipped intentionally — install state is read live from `~/.claude/settings.json`, which is more correct |
| 1.6 Menu + localization | Done | `IDM_AUTO_HIDE = 60`, `IDM_HOOK_STATUS = 61`; all 9 keys across all 6 language files |

## Outstanding work

### A. Phase 1 polish (small, ship before Phase 2)

**A.1 Hook-not-wired fallback tooltip.** When `auto_hide_when_idle` is on and `hook_ever_heartbeat` is still `false`, the widget currently stays visible silently. Plan §1.3 called for surfacing a tray-icon tooltip note so the user understands auto-hide isn't actually doing anything until they install the hook. Implementation:

- In `src/tray_icon.rs`, extend the tray tooltip to accept an optional secondary line.
- In `src/window.rs` wherever the tray tooltip is refreshed, when `auto_hide_when_idle && !hook_ever_heartbeat && hook_installer::status()` is not `OursInstalled`, append a line like "Auto-hide is on but the statusLine hook isn't installed — click to configure."
- Add one localization key `auto_hide_hook_not_installed` across all 6 language files.

**A.2 (Stretch) Chain option in `OtherInstalled`.** Only worth it if we can justify the complexity. The handler would write a small `.bat`/`.ps1` shim that runs the user's existing command first (capturing stdout) then appends ours. Low priority — most users with an existing `ccusage` / `ccstatusline` wouldn't want us chained in anyway.

### B. Phase 2 — Local data source (carried over unchanged)

The prior plan's §2 is still accurate. Reproducing the key decisions here for convenience; refer to section numbers below when implementing.

**B.1 `DataSource` enum and trait.**
- `src/models.rs` — add `DataSource { Api, Local, Hybrid }` and `source: DataSource` on `UsageData`. Settings shape stays backward-compatible (unknown → `Api`).
- `src/poller.rs` — extract current `poll()` into `ApiSource::poll()`. Introduce `trait UsageSource { fn poll(&self) -> Result<UsageData, PollError>; }`. `LocalSource` (in new `src/local_mode.rs`) and `HybridSource` implement it. `do_poll` in `window.rs` dispatches on `AppState::data_source`.

**B.2 Local data source fed by `rate_limits` from the hook.**
- Extend `src/statusline_helper.rs`: after posting the heartbeat, parse `rate_limits.five_hour` and `rate_limits.seven_day` (both optional — absent before the first API response of a session). If present, send via `WM_COPYDATA`. Payload = UTF-8 JSON blob `{ "five_hour": {"u": 23.5, "r": 1738425600}, "seven_day": {"u": 41.2, "r": 1738857600}, "v": "<claude code version>" }`. Use a fixed `dwData` magic (`COPYDATA_RATE_LIMITS`) so the widget ignores unrelated traffic.
- `wnd_proc` handles `WM_COPYDATA`: validate `dwData`, copy the blob out (pointer is only valid for the message duration), parse with serde, update `AppState::last_rate_limits: Option<RateLimits>` plus `rate_limits_captured_at: Instant`.
- Stdout passthrough in the helper: prefer the `rate_limits` values from the payload we just parsed (freshest). Fall back to reading `current_usage.json` only when the payload has no `rate_limits` (first turn of a session). No "read what we just wrote" circularity.
- Persistence across widget restarts: extend `current_usage.json` to include the last-seen `rate_limits`. Trigger a write from the `WM_COPYDATA` path, debounced to ≤1/minute. In-memory `AppState::last_rate_limits` updates real-time; only disk flush is rate-limited. Atomic write via temp + `MoveFileExW`. One file, one writer.
- `src/local_mode.rs` `LocalSource::poll`: return whatever is currently in `AppState::last_rate_limits`, converting `resets_at` (unix epoch seconds) into the same countdown format `ApiSource` uses.
- Staleness: if `rate_limits_captured_at` is older than `local_stale_secs` (default 30 min), surface a "last updated X ago" tooltip but keep rendering the numbers. Offer a menu toggle that writes `"refreshInterval": 60` under `statusLine` in `~/.claude/settings.json`.

**B.3 Handling absent `rate_limits`.** First turn of a new session has no `rate_limits` until the first API response. Widget hydrates from the previous session's persisted `current_usage.json`. If nothing ever cached, `LocalSource::poll` returns `Err(PollError::NoData)` and Hybrid lets the next API call fill the gap.

**B.4 Data source selection.**
- When the hook moves from not-installed → installed and `data_source` is still at its factory default, switch `data_source` to `Local` and save settings. Explicit user picks (Api or Hybrid) are respected.
- Settings submenu gains "Data source" radio: Api / Local / Hybrid.
- `Hybrid` = prefer `LocalSource`; fall back to `ApiSource` when `LocalSource` returns `Err(NoData)` or when `rate_limits_captured_at` is older than `hybrid_fallback_secs` (default 20 min). Successful API responses fold into `last_rate_limits` so subsequent Local reads pick up from there. The cache IS the source of truth here — the API is a periodic sanity check.

**B.5 UI treatment.** Both sources carry server-authoritative percentages. No `~` prefix, no approximation markers. `render_layered` unchanged. Optional tooltip badge (`"5h: 42% · via API"` / `"· via hook"`) as stretch.

**B.6 New files / touched files for Phase 2.**
- *New:* `src/local_mode.rs`.
- *Edit:* `src/statusline_helper.rs` (forward rate_limits via `WM_COPYDATA`), `src/poller.rs` (trait + `ApiSource` extraction), `src/models.rs` (`DataSource`, `RateLimits`, `source` field), `src/native_interop.rs` (`COPYDATA_RATE_LIMITS` const), `src/window.rs` (`WM_COPYDATA` handler, `AppState::last_rate_limits`, debounced flush, hydration, dispatch, radio submenu, refreshInterval toggle).
- *Localization:* new keys for "Data source / Api / Local / Hybrid / last updated {ago} / keep refreshing every 60s / update claude code for local mode" — all 6 language files per the invariant.

### C. Upstream clippy drift (blocker for CI-green)

Six clippy errors on unmodified upstream code, flagged by rust `1.94.0`. Not our bugs, but they fail the repo-wide `cargo clippy --all-targets -- -D warnings` in `CLAUDE.md`.

- `src/poller.rs:536, 641` × 4 — `manual_is_multiple_of`. Trivial swap to `.is_multiple_of()`.
- `src/window.rs:1254` — `paint_content` has 13 args; apply `#[allow(clippy::too_many_arguments)]` above the fn. Real refactor is out of scope here.
- `src/window.rs:1801` — `if_same_then_else` on `max_offset`. Both arms are identical; collapse to a single `let max_offset = tray_left - taskbar_rect.left - widget_width;`. Worth a TODO-flag to whoever originally intended the arms to differ.
- `src/window.rs:2422` — `draw_row` has 8 args; same `#[allow]` treatment.

Also `cargo fmt --check` diff in `src/poller.rs`, `src/tray_icon.rs`. Two options:

1. **One dedicated commit** on this branch titled e.g. "chore: clippy/rustfmt drift on upstream files." Droppable when preparing the upstream PR, or offered upstream as its own small PR.
2. **Scope clippy locally** to files we touched and accept that repo-wide CI stays red until upstream catches up.

**Decision:** go with option 1. Low-risk, keeps the green bar achievable, and if upstream doesn't want it we simply drop the commit.

### D. Phase 3 — Pre-PR trim (user-initiated)

Not auto-triggered. When the user says "let's prep for upstream," run through this list:

**D.1 Ground rules.**
- Base the PR branch on `upstream/main`, not this fork's `main`. `CLAUDE.md` and this `PLAN.md` must not appear in the diff. Sanity check: `git diff upstream/main...HEAD -- CLAUDE.md PLAN.md` is empty.
- Strongly consider splitting Phase 1 (auto-hide + hook) and Phase 2 (local mode) into **two separate PRs**. Phase 1 stands alone and is much easier to review.

**D.2 File-by-file scrutiny.**
- **Iteration scaffolding.** `diagnose::log` calls added during debugging — keep only ones that earn their keep in field reports. Same for stray `eprintln!` / `dbg!`.
- **Dead tunables.** If `stale_session_secs`, `local_stale_secs`, `hybrid_fallback_secs` were never retuned during dogfooding, hardcode them. Every user-visible setting is a maintenance cost.
- **Menu sprawl.** Count new context-menu items. Be especially critical of the refreshInterval toggle and the Data source radio — could Hybrid just be the default with no visible switch?
- **Commentary comments.** Delete anything explaining WHAT the code does. Keep WHY: Win32 gotchas, protocol constraints, bug workarounds.
- **Over-abstracted traits.** If `UsageSource` ends up with three impls where `HybridSource` is the only non-trivial caller, collapse to a free function that dispatches on `DataSource`.
- **Fallback/version logic.** The "Update Claude Code for local mode" version gate is only worth keeping if we know a specific cutoff version. Otherwise delete it and add later with concrete data.
- **Chain-mode installer (A.2).** If not shipped, delete its strings from all six language files and the MessageBox branch.
- **WSL caveats.** Cut any WSL wording that isn't load-bearing.
- **Defensive error handling.** Validate at system boundaries (stdin JSON, `~/.claude/settings.json` parse, `WM_COPYDATA` `dwData`). Drop `if let Err(e) =` / `unwrap_or_default` noise where upstream types already guarantee the invariant.
- **Localization churn.** Every new `Strings` field multiplies across 6 files. For advanced/rarely-seen strings, consider a single English diagnostic over 6 half-accurate translations.
- **Dependency drift.** `cargo tree` should show no new crates beyond `ureq` / `serde` / `serde_json` / `dirs` / `windows` / `winres`. Hand-rolled helpers over new crates (ISO-8601 parser precedent).
- **Binary size.** `cargo build --release && ls -lh target/release/claude-code-usage-monitor.exe` before and after. Release profile: `opt-level="z"`, `lto`, `strip`, `panic="abort"`. Growth > ~50 KB means something is imported-but-unused.
- **`window.rs` size.** Already ~2200 lines. If the diff pushed past ~2500, split cleanly separable chunks (e.g. `WM_COPYDATA` handling, hook-installer UI branches) into their own modules.
- **Commit history.** Squash WIP commits. Each commit = coherent, self-contained, clear message.

**D.3 Pre-PR checks.**
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo fmt -- --check` clean.
- `cargo test` passes (FNV hashing, sweep, JSON merge, ISO-8601).
- Manually re-run the Phase 1 + Phase 2 verification steps on the upstream-based branch.
- Skim the diff against `upstream/main`; everything remaining should directly serve auto-hide (Phase 1) or local mode (Phase 2).

## Verification (carried forward)

### Phase 1 on Windows

1. `cargo build --release` — single exe with both `window::run` and `statusline_helper::run`.
2. Widget Settings → statusLine hook → Install. Confirm `~/.claude/settings.json` is backed up and `statusLine.command` is the absolute exe path + ` statusline`.
3. Start a Claude Code CLI session. Heartbeats arrive (verify via `--diagnose` log showing `WM_APP_HEARTBEAT` hashes). Claude Code's own status bar shows the widget's 5h/7d percentages.
4. Toggle "Only show while Claude Code is running." Widget hides. Start a CLI session → reappears within ~1 s. End the session → disappears after ~6 s.
5. Repeat with a Claude Code session launched from the Anthropic desktop app. Heartbeats same path.
6. Disable hook or point it at a different command; enable auto-hide. Confirm **A.1 fallback tooltip** appears and the widget stays visible.
7. `cargo clippy --all-targets -- -D warnings` and `cargo fmt -- --check` clean (after drift commit C).
8. `cargo test` passes (FNV-1a + sweep tests already in `statusline_helper.rs`; add `hook_installer` JSON-merge + backup tests).

### Phase 2 on Windows

1. Start a Claude Code session and complete one assistant turn. Confirm via `--diagnose` that the widget received a `WM_COPYDATA` with `COPYDATA_RATE_LIMITS` and `AppState::last_rate_limits` matches `ApiSource` output.
2. Flip Data source → Local. Widget immediately renders the in-memory percentages; identical to API mode (not approximate).
3. Pull the network. Keep working in Claude Code. Percentages keep updating on each assistant turn via `WM_COPYDATA`, zero API traffic.
4. Kill the widget, restart while offline. Confirm `current_usage.json` was persisted with the last-seen `rate_limits` and widget hydrates on launch.
5. With Local mode active and Claude Code idle past `local_stale_secs`, confirm "last updated X ago" tooltip appears but numbers keep rendering. Toggle the refreshInterval:60 menu item and confirm it appears in `~/.claude/settings.json`.
6. Switch to Hybrid. Leave Claude Code idle past `hybrid_fallback_secs`. Confirm the next API poll fires and its response updates `last_rate_limits`.

## Risks (carried forward, unchanged)

- **First-turn delay.** Brand-new session has no `rate_limits` until the first API response. Widget hydrates from the previous `current_usage.json`. Only matters on first run of a fresh install — benign.
- **Idle staleness.** statusLine only fires on assistant-message completion (debounced 300 ms). Matches reality (server counters don't change without requests). Tooltip + refreshInterval toggle mitigate.
- **Claude Code version compatibility.** `rate_limits` added in a specific Claude Code release. If version < known-good, Local mode surfaces "Update Claude Code for local mode" and falls back to API. See D.2 — only keep if we have a concrete cutoff.
- **`WM_COPYDATA`.** Sender must outlive `SendMessage` so Windows can marshal the buffer. The helper blocks, so naturally does. Validate `dwData` on receive.
- **AV false positives.** Single signed exe, one subcommand path, no extra processes.
- **Existing user statusLine.** `ccusage`, `ccstatusline`, `starship-claude` etc. — detect and offer Replace/Cancel (and maybe Chain per A.2). Never silently overwrite.
- **Path quoting.** Hook install embeds absolute exe path into JSON; test with spaces and non-ASCII.
- **Backup cleanup.** Timestamped `settings.json.ccum-backup-<unix>` files accumulate. Document in Settings; offer manual "Clean old backups" later if needed.
