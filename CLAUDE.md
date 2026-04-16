# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A Windows-only Rust binary that draws a small widget into the Windows taskbar showing Claude Code's 5-hour and 7-day usage percentages plus countdowns to reset. Also provides a system tray icon. Target platform is `windows-latest`; the code will not compile or run on Linux/macOS because it depends heavily on the `windows` crate and `std::os::windows`.

## Commands

All builds must run on Windows.

```powershell
cargo build --release                      # produces target/release/claude-code-usage-monitor.exe
cargo build                                # debug build
cargo check                                # fast type-check without producing a binary
cargo clippy --all-targets -- -D warnings  # lint
cargo fmt                                  # format
```

There are currently no unit or integration tests (`cargo test` runs the empty suite). When adding tests, prefer placing platform-independent helpers (e.g. in `poller.rs`: `parse_iso8601`, `format_countdown_from_secs`, `is_version_newer`) behind `#[cfg(test)]` modules so they can be exercised without the Windows runtime.

### Runtime flags

- `claude-code-usage-monitor --diagnose` writes a log to `%TEMP%\claude-code-usage-monitor.log` (see `diagnose.rs`).
- `claude-code-usage-monitor --apply-update <target> <source> <pid>` is the updater-helper entry point; not for manual use (see `updater::handle_cli_mode`).

Settings live at `%APPDATA%\ClaudeCodeUsageMonitor\settings.json` and are loaded/saved by `window::{load_settings, save_settings}`.

## Releases

Tags matching `v*` on `main` trigger `.github/workflows/release.yml`, which builds the release binary, publishes a GitHub Release with `claude-code-usage-monitor.exe` attached, and submits a WinGet manifest update for `CodeZeno.ClaudeCodeUsageMonitor` via `wingetcreate` (gated on the `WINGETCREATE_GITHUB_TOKEN` secret). The version embedded in the PE resource comes from `Cargo.toml` via `build.rs`, so version bumps happen there.

## Architecture

The entry point is `main.rs`, which is annotated `#![windows_subsystem = "windows"]` so no console appears. It dispatches to either the updater CLI mode or `window::run`.

### Module layout

- `main.rs` — startup, arg parsing, `--diagnose` bootstrap, updater CLI dispatch.
- `window.rs` (~2200 lines) — the heart of the app. Owns the Win32 message loop, global `AppState` behind a `Mutex<Option<AppState>>`, the layered window that renders into the taskbar, all context menus, DPI handling, settings persistence, poll thread orchestration, and `wnd_proc`.
- `poller.rs` — fetches usage. Reads OAuth credentials from `~/.claude/.credentials.json` (and from any WSL distro via `wsl.exe -d <distro> -- sh -lc 'cat ~/.claude/.credentials.json'`), refreshes expired tokens by invoking the `claude` CLI with `-p .`, then hits `https://api.anthropic.com/api/oauth/usage` and falls back to `POST /v1/messages` to parse `anthropic-ratelimit-unified-*` headers. Also contains the minimal ISO-8601 parser (intentional, to avoid pulling in `chrono`/`time`).
- `tray_icon.rs` — draws the notification-area badge (color-interpolated fill based on 5h utilization) and owns the `TrayAction` enum returned to `window.rs` for left-click/right-click handling.
- `updater.rs` — GitHub release check, self-update flow for portable installs (spawns a copy of itself as `updater-helper.exe` with `--apply-update` so the main exe can be replaced), and `winget upgrade` flow for WinGet installs. `current_install_channel()` decides which path to take based on the exe's location.
- `native_interop.rs` — thin, typed wrappers over Win32 APIs (taskbar lookup, window reparenting, `SetWinEventHook`, colorref helpers, widestring conversion) plus shared `WM_APP_*` message IDs and timer IDs.
- `theme.rs` — reads `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme` to pick a palette.
- `models.rs` — `UsageData` / `UsageSection` DTOs shared across modules.
- `diagnose.rs` — opt-in file logger gated by `--diagnose`; `diagnose::log` is a no-op until `init()` succeeds, so it is safe to sprinkle.
- `localization/` — one module per language (`english.rs`, `spanish.rs`, `french.rs`, `german.rs`, `japanese.rs`, `korean.rs`) each exposing a `STRINGS: Strings` and a `UPDATE_VIA_WINGET_LABEL` constant. The `Strings` struct in `localization/mod.rs` is the single source of truth for translatable keys — adding a field there requires updating every language file. System language is auto-detected via `GetUserPreferredUILanguages` / `GetUserDefaultUILanguage` and can be overridden through the right-click menu.

### Key runtime patterns

- **Single-instance**: `window::run` acquires `Global\ClaudeCodeUsageMonitor` via `CreateMutexW` and silently exits if it already exists.
- **Taskbar embedding**: the window is created as `WS_POPUP | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, then reparented into `Shell_TrayWnd` via `native_interop::embed_in_taskbar`, which also flips `WS_POPUP` to `WS_CHILD`. A `SetWinEventHook` on `EVENT_OBJECT_LOCATIONCHANGE` keeps it aligned when the taskbar moves/resizes.
- **Rendering**: the widget is drawn via `UpdateLayeredWindow` (see `render_layered` in `window.rs`). All pixel dimensions are authored at 96 DPI and scaled through the `sc()` helper; `CURRENT_DPI` is refreshed on `WM_DPICHANGED`.
- **Polling**: a background thread driven by `TIMER_POLL` / `TIMER_RESET_POLL` / `TIMER_COUNTDOWN` / `TIMER_UPDATE_CHECK` (IDs in `native_interop.rs`) posts `WM_APP_USAGE_UPDATED` back to the main thread. All state mutation goes through `lock_state()`, which transparently recovers from a poisoned mutex.
- **Threading boundary**: `HWND` is wrapped in `SendHwnd` (a `Copy` isize) so it can cross threads for `PostMessage`. Never pass raw `HWND` across threads.
- **HWND validity**: always treat returned `HWND` as possibly-null; `native_interop` helpers already map `HWND::default()` → `None`.

### Credentials & the Claude CLI

`poller::read_credentials` collects candidates from both the Windows user profile and every enumerated WSL distro (`wsl.exe -l -q`, output is UTF-16LE and handled by `decode_wsl_text`), then picks the first non-expired one via `choose_best_credentials`. Refreshes are performed by invoking `claude -p .` with `CLAUDECODE`/`CLAUDE_CODE_ENTRYPOINT` env vars cleared, so the spawned CLI does not think it is being run by another Claude Code session. Any new subprocess on Windows should pass `creation_flags(CREATE_NO_WINDOW)` (`0x08000000`) to avoid a flashing console window.

### Dependency philosophy

Dependencies are deliberately minimal (`ureq` with `native-tls`, `serde`, `serde_json`, `dirs`, `windows`, `winres`) and the release profile is tuned for small binary size (`opt-level = "z"`, `lto = true`, `strip = true`, `panic = "abort"`). Prefer hand-rolled helpers over pulling in new crates; the ISO-8601 parser in `poller.rs` is there for exactly this reason.

## Fork workflow

This repository is a fork of `CodeZeno/Claude-Code-Usage-Monitor`. Features developed here are intended to be proposed back upstream via pull request.

**`CLAUDE.md` is fork-local and must never be included in upstream PRs.** When preparing a branch for an upstream PR:

- Base the PR branch on `upstream/main` (not this fork's `main`), or rebase/cherry-pick feature commits onto an upstream-based branch so `CLAUDE.md` is not part of the diff.
- Do not add `CLAUDE.md` changes to commits that are meant to go upstream.
- If a PR branch accidentally contains `CLAUDE.md`, drop that commit (or revert the file) before opening the upstream PR.

Day-to-day feature branches should still be based on this fork's `main` so they inherit `CLAUDE.md` automatically.
