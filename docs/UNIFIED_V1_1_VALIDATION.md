# AI-Hands v1.1.0-unified.1 validation

This report identifies the exact local publication artifacts prepared for source review. The standalone Rust executable is signed; the Windows installer is not signed or released.

## Final Rust binary

- File: `dist/hand.exe`
- Source-exact unsigned SHA-256 before signing: `F3C8C8A83A36E89BD19C64076EBB0432B2534922C9DB521BDB9322AA11900529`
- Final signed SHA-256: `CD218D6356830AE7A926CD2E78760E726A5BF8B5F1381457124ED19A485B99C4`
- Final signed size: 21,768,976 bytes
- Signature state: Valid and timestamped
- Embedded-string checks: no workstation-specific username path, private user path, old live-binary hash, private coverage filename, hardcoded Node executable, or private JavaScript-helper path. Generic, non-user-specific fallback strings from the public path-discovery dependency remain by design.
- Dependency check: one vendored `vision-core` package is shared by Hands and the vendored browser library

## Four-monitor acceptance

Both runs used the unchanged strict acceptance suite and the exact final signed binary hash above. Each found four monitors, exercised fixed-and-locked scope on every monitor, rejected mismatched or unscoped actions, verified browser and QR bindings, blocked native plugins under scope, propagated rejected UIA batch actions, and validated unique decodable PNG captures.

| Run report | Report SHA-256 | Result |
|---|---|---|
| `monitor-smoke-20260715_014118_828_108988.json` | `38F7BD0D327E5C39E57B1F45B6CB7D9BFD61FEED3A01AE15946BC965D350B634` | PASS, 13 of 13 |
| `monitor-smoke-20260715_014219_978_87604.json` | `81C0D87BEAD8DDCC24C61D52384B5982B253480D3EA13565D5DCB2460D55D0CE` | PASS, 13 of 13 |

The four physical stable IDs were identical across both runs. The IDs themselves are intentionally omitted from this public report.

## Rust and plugin checks

- Unit tests: 371 discovered; 370 passed, 0 failed, 1 ignored because the optional example plugin was not built.
- Clippy: all targets passed with warnings denied.
- Format check: passed.
- RustSec audit: passed after updating `crossbeam-epoch` to 0.9.20. Two Linux-only `quick-xml` advisories are explicitly ignored by `cargo audit` because this Windows build reaches them only through trusted Wayland/XCB build-time metadata.
- Cargo Deny CI policy: license, duplicate-ban, and source checks passed.
- Hook policy tests: 5 passed.
- Codex plugin validator: passed.
- Claude plugin validator: passed.
- Grok plugin validator: passed.
- Both bundled skill validators: passed.

## Windows installer

- File: `dist/AIHands-Setup-1.1.0-unified.1-arm64.exe`
- SHA-256: `7BBD2E9A51E2F17755E6A6BAC30E1D1123ED0417D8DFC22892781980244B7956`
- Size: 7,114,012 bytes
- Signature state: unsigned
- Embedded Rust binary SHA-256: `920C5C64DC83399BC86DD917A45DC686B8FC142F875F1C08108D2C9EDBD45E7E`
- Required payloads: 9 of 9 present in the actual test install
- PATH: unchanged when the selectable PATH task was disabled
- AI and hook configuration: not edited
- Browser detection: existing 64-bit Chrome detected; no false missing-browser warning
- Uninstall: exit code 0 and test installation directory removed
- Package string checks: no workstation username path or old live-binary hash

This installer predates the final dependency update and signature. It copied the complete per-AI guide with its native Unicode clipboard path and logged the success popup; the acceptance harness restored the pre-test clipboard afterward. If native clipboard access is unavailable, the installer also tries original-user PowerShell fallbacks and truthfully points to the installed guide when every method fails.

## Release boundary

The source and standalone signed Rust executable are ready for publication review. The installer is held until it is rebuilt around the final signed Rust payload, signed itself, and reruns install/uninstall acceptance. An x64 decision also remains open. Inno Setup's base license permits commercial use; the vendor requests commercial users purchase a license but states that purchase is not strictly required. Record the publisher's support and compliance choice before commercial distribution; it is not a technical build blocker. See the [official commercial-license guidance](https://jrsoftware.org/isorder.php). No live AI configuration was edited, no live Hands binary was replaced, and no shared service was restarted.
