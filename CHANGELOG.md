# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## v1.1.0-unified.1 - 2026-07-15 - Unified preview

### Added

- Ability-based union profiles (`default`, `full`, `strict`, and `compatibility`) with a machine-readable capability catalog and no duplicate registrations.
- Central fail-closed monitor scope with fixed physical stable IDs and locking for unattended automation.
- Collision-resistant screenshot filenames across same-process, same-second, and concurrent-process captures.
- Optional cross-host plugin with Codex and Claude manifests, ability-routed skills, per-AI instructions, and inert opt-in hook templates.
- Inno Setup package that stages the Rust binary, plugin, skills, prerequisites, helper files, and a per-AI guide without silently editing AI or hook configuration.

### Changed

- Vendored browser, UIA, and vision libraries so the audited source used by the binary is present in this repository.
- Removed workstation-specific screenshot paths from vision-core. Default captures now resolve the signed-in user's Pictures directory at runtime and use unique names.
- Made the optional Node extraction helper portable through PATH or `AIHANDS_NODE_EXE`, `AIHANDS_JS_EXTRACT_SCRIPT`, and `AIHANDS_NODE_PATH`.
- Disabled execution of untrusted page scripts in the optional jsdom parser. Use attached-browser DOM tools when live JavaScript execution is required.

### Validation

- 370 unit tests passed and one optional example-plugin test was ignored.
- Clippy passed for all targets with warnings denied.
- Two strict four-monitor acceptance runs passed all 13 checks on the exact publication-sanitized binary; physical stable IDs were identical across runs.
- Installer acceptance confirmed the embedded binary hash, nine required payloads, unchanged PATH when disabled, no AI-config edits, no false Chrome warning, clean uninstall, and truthful clipboard-unavailable fallback.


## v1.0.1 — 2026-05-15 — Dep-bump security release

Resolves Dependabot alerts inherited from the AIWander/hands → AIWander/AI-Hands
mirror-rename on 2026-05-15. No feature changes.

### Resolved vulnerabilities

- **HIGH**: openssl 0.10.78 → 0.10.79 (GHSA-xp3w-r5p5-63rr) — undefined behavior
  in `X509Ref::ocsp_responders` for certificates with non-UTF-8 OCSP URLs.
- **MODERATE**: openssl 0.10.78 → 0.10.79 (GHSA-xv59-967r-8726) — heap buffer
  overflow when encrypting with AES key-wrap-with-padding. Same bump as the
  HIGH alert above; one openssl-sys minor bump resolves both.
- **LOW**: lru 0.12.5 → 0.16.4 (GHSA-rhfx-m35p-ff5j) — `IterMut` violates
  Stacked Borrows by invalidating an internal `HashMap` pointer (unsound, not
  yet exploited). Required bumping `rqrr` 0.7 → 0.10 since rqrr 0.7.1 pinned
  `lru ^0.12`; rqrr 0.10.1 depends on `lru ^0.16` and the public API surface
  used by AI-Hands (`PreparedImage::prepare_from_greyscale`, `detect_grids`,
  `Grid::decode`) is unchanged.

### Build

- Both Windows targets rebuilt cleanly with `cargo check --release` passing.
- x64 hands.exe: 22.55 MB (vs v1.0.0 23.65 MB).
- ARM64 hands.exe: 19.01 MB (vs v1.0.0 19.95 MB).

## v1.0.0 — 2026-05-15 — AI-Hands launch

Rebrand of AIWander/hands → AIWander/AI-Hands. Same Rust codebase, fresh
versioning. The hands.exe binary name and `hands:*` MCP tool prefix are
unchanged — existing MCP configs in Claude Desktop, Claude Code, Cowork,
Codex CLI, Gemini CLI, LM Studio keep working without edits.

### New in this release

- **New tool: `vision_screenshot_hidden_window`** — always-PrintWindow API
  captures a window's pixels without bringing it to the foreground. Replaces
  the `behind=true` mode of `window_screenshot`.
- **`window_title` parameter on `hands_capture`** — focus a named window via
  UIA before routing the capture.
- **`offset_x`/`offset_y` on `hands_click`** — when non-zero, every rung of
  the 7-rung click ladder resolves the element by its native method then
  coord-clicks at bbox.center + offset. When both zero, ref/selector
  click is preserved on rungs 1-4 for reliability.

### Deprecation markers (handlers preserved)

- `find_and_click` → use `hands_click` (offsets now available on every rung)
- `retry_click` → use `browser_click` with retry option (coming in next browser-mcp release)
- `read_screen_text` → use `vision_screenshot_ocr` (optional `window_title` via `hands_capture`)
- `type_into_window` → use `hands_type` (already includes focus verification and chunked typing)
- `window_screenshot` (default mode) → use `vision_screenshot`; for hidden-window capture use `vision_screenshot_hidden_window`

### Note on versioning

This is v1.0.0 of `AIWander/AI-Hands`, the renamed successor repo. The previous
repo (`AIWander/hands`) reached v1.3.5 before the rename; that history is
preserved in this repo's git log but v1.x.x tags have been stripped so the
tag list starts clean at v1.0.0. The codebase here is the v1.3.5 baseline
plus today's source improvements layered on top.

---

## v1.3.5 - 2026-05-01

*(Last release under the old AIWander/hands name. History preserved below for context.)*


### Changed

- **cargo fmt** — reformatted `src/atomic.rs` and `src/main.rs` to pass `cargo fmt --all -- --check`.
- **cargo clippy cleanup** — fixed 32 errors across 11 distinct lints (`collapsible_if`, `if_same_then_else`, `manual_map`, `manual_contains`, `single_match`, `needless_range_loop`, `needless_borrows_for_generic_args`, `unnecessary_cast`, `unnecessary_map_or`, `useless_format`, `regex_creation_in_loops`). Two regex compilations hoisted out of loop in `browser_learn_api`.
- **README: corrected "Playwright" to chromiumoxide** — Hands uses [chromiumoxide](https://github.com/mattsse/chromiumoxide), a pure-Rust CDP client, not Playwright. Fixed 28 stale references across README.md, CHANGELOG.md, docs/, and skills/.
- **README: collapsed install sections** — merged separate Windows x64 and ARM64 install sections into one.
- **README: tool count 116 to 117** — corrected to match actual source tool count.
- **README: expanded capability inventory** — added sections for browser compatibility, a11y-first targeting, multi-context isolation, network interception, API discovery, cross-server graduation pipeline, authorized credential/MFA workflows, UIA window management, vision template matching, and meta-tool escalation ladder.
- **README: trimmed "Related repos" header** — removed `cpc-paths` and `cpc-breadcrumbs` (shared library crates, not user-facing servers).
- **README: email mailto link** — plain text email replaced with clickable `mailto:` link.
- **Version alignment** — Cargo.toml, README.md, and CHANGELOG.md all at v1.3.5.

### Added

- **GitHub Actions release workflow** — `v*` tag push builds x64 (windows-latest) + ARM64 (windows-11-arm native) binaries, attaches to draft release as `hands-vX.Y.Z-x64.exe` / `hands-vX.Y.Z-aarch64.exe`.
- **SECURITY.md** — security policy and reporting instructions.
- **Platform-split install docs** — README install section now covers both x64 and ARM64 in a single section with binary naming convention.

## v1.3.4 - 2026-04-29

### Changed
- ci: bump GitHub Actions versions to latest (Node.js 20 deprecation)

## v1.3.3 - 2026-04-19

### Changed

- **Phase D: compile-time ZST AtomicTool dispatch** — Replaced all runtime string-based UIA tool dispatch in meta-tools with zero-sized-type (ZST) `AtomicTool` handles resolved at compile time. 11 UIA tools wrapped (`UiaClick`, `UiaType`, `UiaFindElement`, `UiaFocusWindow`, `UiaKeyPress`, `UiaShortcut`, `UiaReadValue`, `UiaScroll`, `UiaGetState`, `UiaListWindow`, `UiaWatch`). 7 meta-tool files refactored (`app_action`, `capture`, `click`, `find`, `qr_scan`, `type_text`, `verify`). 27 call sites replaced. 245/245 tests pass.

### Added

- **`src/atomic.rs`** — New module defining the `AtomicTool` trait and ZST wrappers for all UIA tools, plus browser-side atom helpers. Provides compile-time guarantees that tool names match canonical MCP tool names.
- **Browser compatibility module** — Optional browser compatibility adjustments for authorized automation testing.

## v1.3.2 - 2026-04-17

### Changed

- **Clippy + dead_code + unused cleanup** -- removed 3 crate-level `#![allow(...)]` suppressions. Added 60+ targeted item-level `#[allow(...)]` annotations with justification. 22 supplemental mechanical fixes in `src/meta/*` modules.

## v1.3.1 - 2026-04-17

### Changed
- Migrated HTTP dashboard endpoint from hyper to tiny_http for reduced binary size and simpler stack
- Duration tracking for tool calls in dashboard status responses
- Credential redaction in dashboard output (API keys, tokens masked)
- Field name alignment across all dashboard JSON responses
- Metadata cleanup in Cargo.toml (description, license, repository fields)

### Fixed
- Mojibake in documentation files (curly quotes, em-dashes replaced with ASCII equivalents)

## v1.3.0 - 2026-04-16

### Changed
- Bumped Cargo.toml from 0.1.0 to 1.3.0 to match tag history (was stuck at 0.1.0 despite tags v1.1.1, v1.2.1, v1.2.2, v1.3.0-dev)
- Swapped `browser-mcp` path dep to git tag pin (`AIWander/browser-mcp @ v0.1.1`) -- resolves CRITICAL-1
- Swapped `vision-core` path dep to git tag pin (`AIWander/vision-core @ v0.1.0`) -- resolves CRITICAL-1
- Swapped `uia-mcp` path dep to git tag pin (`AIWander/uia-mcp @ v1.0.0`) -- 4th unpublished path dep discovered during F5 attempt (audit missed it), published via F7
- Committed Cargo.lock for reproducible CI builds -- resolves CRITICAL-3
- README.md License section: MIT -> Apache-2.0 (final residue from MIT->Apache-2.0 migration)
- Added `license = "Apache-2.0"` + `repository` + `description` to Cargo.toml

### Notes
- First version of hands that builds cleanly as a standalone public clone without the rust-mcp workspace.

## [Unreleased] -- 1.3.0-dev

### Changed
- License changed from MIT to Apache-2.0; `Cargo.toml` updated to `license = "Apache-2.0"`.
- Add legacy-fallback path resolution for instrumentation logs. Existing `C:\CPC\logs\` (if present with `hands_meta*.jsonl` data) continues to be used; new installs use `cpc_paths::data_path("hands")` default. Resolved once at startup via `OnceLock`.

### Added
- **Two-Tier Storage** section in `docs/per_machine_setup.md` -- documents Volumes vs local-data distinction, what not to sync, legacy paths, and second-machine setup walkthrough.
- **`hands_health` MCP tool** -- diagnostic health check exposing `cpc_paths::health_check()` (path resolution for Volumes, install, backups) plus browser, vision, and UIA subsystem probe results.
- **`cpc-paths` dependency** (v0.1.0) -- portable path discovery library, pinned to git tag.
- `meta::health::hands_health()` -- public function aggregating cpc-paths + subsystem probes.
- 3 new unit tests in `meta::tests_phase_c` for hands_health shape, paths fields, and subsystem status values.

## [0.1.0] - 2026-03-30

### Added

- Initial release with 116 MCP tools across 5 automation tiers
- Browser automation via chromiumoxide CDP (navigate, click, fill, screenshot, eval, and more)
- Windows UI Automation via COM (find elements, click, type, read values, manage windows)
- Vision tier: screenshot capture, OCR text extraction, template matching, image diff
- Accessibility snapshot support for structured page inspection
- XPath selectors with auto-wait for reliable element targeting
- Batch operations (`browser_batch`, `uia_batch`) for multi-step sequences in a single call
