# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — 1.3.0-dev

### Added
- **`hands_health` MCP tool** — diagnostic health check exposing `cpc_paths::health_check()` (path resolution for Volumes, install, backups) plus browser, vision, and UIA subsystem probe results.
- **`cpc-paths` dependency** (v0.1.0) — portable path discovery library, pinned to git tag.
- `meta::health::hands_health()` — public function aggregating cpc-paths + subsystem probes.
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
