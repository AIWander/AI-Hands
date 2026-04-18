# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
- Swapped `browser-mcp` path dep to git tag pin (`josephwander-arch/browser-mcp @ v0.1.1`) -- resolves CRITICAL-1
- Swapped `vision-core` path dep to git tag pin (`josephwander-arch/vision-core @ v0.1.0`) -- resolves CRITICAL-1
- Swapped `uia-mcp` path dep to git tag pin (`josephwander-arch/uia-mcp @ v1.0.0`) -- 4th unpublished path dep discovered during F5 attempt (audit missed it), published via F7
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

- Initial release with 71 MCP tools across 3 automation tiers
- Browser automation via Playwright CDP (navigate, click, fill, screenshot, eval, and more)
- Windows UI Automation via COM (find elements, click, type, read values, manage windows)
- Vision tier: screenshot capture, OCR text extraction, template matching, image diff
- Accessibility snapshot support for structured page inspection
- XPath selectors with auto-wait for reliable element targeting
- Batch operations (`browser_batch`, `uia_batch`) for multi-step sequences in a single call
