# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

This repository contains a working Tauri v2 + Svelte 5 implementation. `DESIGN.md` remains the product/behavior specification, and source changes should keep it in sync when behavior changes.

## Authoritative source

`DESIGN.md` is the source of truth. Before making recommendations or writing code, read it. Any proposal that contradicts `DESIGN.md` must either (a) update `DESIGN.md` first, or (b) be raised to the user as a design-change question.

The design has already gone through several decision rounds — do not re-litigate decisions marked in `## 確定事項`. Examples of already-closed questions:

- Target is **`.prproj` only**; do not add configurable extension lists.
- Filter constants (`^\d{6}\(` regex, `Auto-Save` exclusion, 300-second Drive wait, `_Latest.prproj` suffix) are **hardcoded in source**, not in `config.json`. User-facing config is deliberately minimal: user-editable are only `source` / `destination` / `scheduleTime` / `autoStart` / `excludedFolders` / `excludedFolderNames`; `lastRunAt` / `lastSummary` are persisted state.
- **Resident mode only** — no CLI subcommands, no Task Scheduler integration.
- Catch-up-on-startup is **always on** (not a toggle).
- Config/logs live in `<exe_dir>/data/` with fallback to `%USERPROFILE%\yuru-auto-backup-gdrive\`. Do not use `%APPDATA%`.

## What the app does (one-paragraph summary)

Windows-only Tauri v2 desktop app. Once per day at a user-configured time, scans a source folder for `.prproj` files under folders whose name matches `^\d{6}\(`, excludes anything under `Auto-Save`, and copies each match to a Google Drive-synced destination folder as `<BaseName>_Latest.prproj` (flat, overwriting). Google Drive for desktop handles the actual cloud upload. If the destination path isn't visible yet (Drive not started), it polls for up to 5 minutes before giving up. If the PC was off at the scheduled time, the next launch runs it immediately.

## Architecture notes that span multiple components

- **`AppDir::resolve()` is the entrypoint for all persistence.** Both `ConfigStore` and `Logger` must go through it. The portable-first / user-home-fallback logic lives in one place so installation layout is transparent to callers.
- **Filter constants are shared between `BackupJob` and any future test fixtures.** They live in `backup.rs` (section 8.3 of `DESIGN.md`). If you add a test for filtering behavior, import the constants — don't redefine them.
- **`DrivePathDetector` is best-effort and must never fail the user flow.** It returns a candidate list; the UI always falls back to a plain folder picker if detection yields zero results. Callers should treat empty results as normal, not as an error.
- **Atomic copy is required** (`.part` → `rename`), not optional. The destination lives on a Drive-synced folder, and a partial file being picked up by the sync client would pollute the cloud copy.
- **Scheduler and the catch-up check are one mental model, not two.** On startup: if `lastRunAt` is not today and today's `scheduleTime` has passed, run immediately, then arm the timer for tomorrow. Don't add a separate "missed run" concept.

## Reference material

- `DESIGN.md` section 15 ("既存 PowerShell スクリプトとの対応表") maps every behavior in the original `backup_prproj.ps1` to its new home in the app. Useful when verifying parity with the existing script that this app replaces.

## Conventions for editing `DESIGN.md`

- Section numbers are referenced from other sections (e.g., "see 8.3"). When inserting a new section, update all downstream references or renumber carefully.
- New decisions go into `## 確定事項` with a one-line summary; don't leave them buried in prose.
- When a previously-open question is resolved, move it from `未確定事項` to `確定事項` rather than just deleting it — preserving the decision trail helps future reviewers understand why things are the way they are.
