# Session handoff: claude/statusline-hook

**Read first:** `CLAUDE.md` (project conventions, fork-policy) and `PLAN.md` (the full design doc this branch is implementing).

## Where we are

Branch `claude/statusline-hook` (this is the working branch — develop directly here). Tip is `7f2adad`.

The branch implements two related features that the prior plan calls Phase 1 (auto-hide) and Phase 2 (Local data source):

1. A `statusline` subcommand that the same exe exposes for Claude Code's `statusLine` hook. The hook posts `WM_APP_HEARTBEAT` to the running widget and forwards `rate_limits` via `WM_COPYDATA`.
2. An "Only Show While Claude Code is Running" auto-hide toggle, driven by the heartbeat above.
3. A "statusLine hook" install/uninstall UI in Settings that backs up `~/.claude/settings.json` before merging.
4. A `DataSource { Api, Local, Hybrid }` choice in Settings:
   - **Api** — original behavior, polls `https://api.anthropic.com/api/oauth/usage`.
   - **Local** — reads in-memory `last_rate_limits` posted by the hook. Zero network.
   - **Hybrid** — prefers Local while hook data is fresh (< `hybrid_fallback_secs`, default 20 min), falls back to Api and folds the API response back into `last_rate_limits` so subsequent Local reads stay authoritative.
5. `current_usage.json` persists `last_rate_limits` (debounced ≤1/min) so LocalSource has data immediately after restart, before the first heartbeat of a new session.
6. A "Refresh statusLine every 60 s" toggle (only shown when our hook is installed) that writes `statusLine.refreshInterval = 60` into `~/.claude/settings.json`.

When the user installs our hook from scratch and `data_source` is still at the factory default (`Api`), we auto-switch to `Local` and surface a confirmation in the install success message. Explicit user picks are respected.

## Commit log on this branch

```
7f2adad  Drop redundant `ref` in statusline_helper rate_limits send
f7abe2b  Persist last_rate_limits in current_usage.json and hydrate on startup
352b04c  Hybrid data source + Data Source menu + refreshInterval toggle
5da67e9  Local data source fed by WM_COPYDATA from the statusLine hook
0bf8c22  Introduce UsageSource trait and DataSource enum
f04ed68  Show tray hint when auto-hide is on but hook is not installed
f4ef251  chore: clippy and rustfmt drift on upstream files
1b47a30  Add fork-local PLAN.md
854258c  Merge pull request #1 (earlier web-session work)
27bb98f  Fix clippy::bool_assert_comparison in hook_installer test
3267623  Add statusLine helper and auto-hide-when-idle toggle
16e0158  Document fork-local CLAUDE.md policy
ecdb4ec  Add CLAUDE.md for Claude Code
```

`f4ef251` is the upstream-drift commit (clippy `manual_is_multiple_of` × 4, `if_same_then_else` collapse, two `#[allow(clippy::too_many_arguments)]`). Drop or cherry-pick it independently when preparing the upstream PR — it touches files this feature otherwise leaves alone.

## Files touched

New:
- `src/statusline_helper.rs` — `statusline` subcommand: stdin parse → heartbeat + WM_COPYDATA + stdout echo.
- `src/hook_installer.rs` — install/uninstall/status of `~/.claude/settings.json`'s statusLine entry, plus `set_refresh_interval`.
- `src/local_mode.rs` — `LocalSource` impl reading `AppState::last_rate_limits`.
- `PLAN.md` — fork-local plan doc.
- `CLAUDE.md` — fork-local conventions.

Modified:
- `src/main.rs` — subcommand dispatch.
- `src/window.rs` — `AppState`, `SettingsFile`, `wnd_proc` (`WM_COPYDATA`, `WM_APP_HEARTBEAT`), `apply_visibility`, hook install UI, Data Source submenu, refreshInterval toggle, `poll_via_current_source` dispatcher with Hybrid logic and fold-back.
- `src/native_interop.rs` — `WM_APP_HEARTBEAT`, `WIDGET_WINDOW_CLASS`, `COPYDATA_RATE_LIMITS` magic.
- `src/poller.rs` — `UsageSource` trait, `ApiSource`, `PollError::NoData`, `iso8601_to_unix` helper.
- `src/models.rs` — `DataSource`, `RateLimits`, `RateLimitBucket`, `source` field on `UsageData`.
- All six `src/localization/*.rs` files + `src/localization/mod.rs` — 8 new keys: `auto_hide_hook_not_installed`, `data_source`, `data_source_api`, `data_source_local`, `data_source_hybrid`, `refresh_interval_60`, `data_source_switched_to_local`, `refresh_interval_set`.
- `Cargo.toml` — added `Win32_System_DataExchange` feature for `COPYDATASTRUCT`.

## What still needs doing

### Verification on Windows (highest priority)
The web session never ran the build. Before any further work, run on Windows from the repo root:

```powershell
git pull --ff-only origin claude/statusline-hook
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Then the manual flows in `PLAN.md` "Verification" section (both Phase 1 and Phase 2 lists). If anything fails, the issue is almost certainly a Windows-only API mismatch — the web session has no way to compile against the `windows` crate.

**Conflict with stable install.** The user runs the stable v1.3.1 alongside dev. Both use the `Global\ClaudeCodeUsageMonitor` mutex, the same `%APPDATA%\ClaudeCodeUsageMonitor\settings.json`, and the same `%LOCALAPPDATA%\ClaudeCodeUsageMonitor\current_usage.json`. Always exit stable from its tray menu before launching the dev build, and back up `settings.json` before testing. The dev build adds new fields (`auto_hide_when_idle`, `stale_session_secs`, `data_source`, `hybrid_fallback_secs`) that stable would silently drop on next save.

### Likely follow-ups after verification

1. **Field-test outcomes feed back into the plan.** If `hybrid_fallback_secs` or `stale_session_secs` defaults feel wrong, retune. If they never get retuned, hardcode them per `PLAN.md` §D.2 "dead tunables."
2. **§A.2 Chain installer (stretch).** Skipped — would let `OtherInstalled` users keep their existing statusLine and chain ours after it. Low value if most ccusage/ccstatusline users wouldn't actually want both.
3. **§B.5 source badge in tooltip (stretch).** Skipped — `· via API`/`· via hook` suffix in the tray tooltip. Tray text is capped at 127 chars and we already use it for the hook hint, so this competes for space.
4. **§D pre-PR trim.** Not auto-triggered. Wait for the user to say "let's prep for upstream." Then base a fresh branch on `upstream/main` (NOT this fork's `main`) so `CLAUDE.md` and `PLAN.md` don't appear in the diff, drop the upstream-drift commit (`f4ef251`) — or offer it upstream as its own chore PR — and run through the file-by-file scrutiny list in `PLAN.md` §D.2. Strongly consider splitting Phase 1 (auto-hide + hook) from Phase 2 (local mode) into two PRs.

## Hand-off conventions

- **Develop on `claude/statusline-hook` directly.** Earlier sessions were forked onto throwaway harness branches (`claude/windows-compile-testing-HHH0T`); that's a quirk of Claude Code on the web. A local session can commit straight to this branch.
- **Do not touch upstream files in feature commits.** If clippy/rustfmt complains about a file this feature didn't introduce, either add to the existing `f4ef251` drift commit (and update its message) or reject the lint with a comment noting why.
- **Localization invariant.** Every `Strings` field added to `localization/mod.rs` requires a value in all six language files. Never half-translate.
- **Never silently mass-rebuild.** `window.rs` is ~2500 lines now. If you find yourself wanting to refactor unrelated code to "improve" things, stop — keep the diff scoped to the feature.
- **Subprocess spawns must pass `creation_flags(CREATE_NO_WINDOW)` (`0x08000000`)** to avoid console flashes.
- **Cross-thread `HWND`** — wrap in `SendHwnd`, never raw `HWND`.

## Pointers

- Full design and Phase 3 trim checklist: `PLAN.md`.
- Project conventions, runtime flags, settings paths, dependency philosophy: `CLAUDE.md`.
- Upstream repo: `CodeZeno/Claude-Code-Usage-Monitor`. This is a fork at `tjmchenry/Claude-Code-Usage-Monitor`.
