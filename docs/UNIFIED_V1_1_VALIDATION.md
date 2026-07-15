# AI-Hands v1.1.0-unified.1 validation

This report identifies the exact local publication artifacts prepared for review. No artifact in this report is signed or published.

## Final Rust binary

- File: `dist/hand.exe`
- SHA-256: `920C5C64DC83399BC86DD917A45DC686B8FC142F875F1C08108D2C9EDBD45E7E`
- Size: 21,748,224 bytes
- Signature state: unsigned
- Embedded-string checks: no workstation username path, private Drive path, old live-binary hash, private coverage filename, hardcoded Node executable, or private JavaScript-helper path
- Dependency check: one vendored `vision-core` package is shared by Hands and the vendored browser library

## Four-monitor acceptance

Both runs used the unchanged strict acceptance suite and the exact final binary hash above. Each found four monitors, exercised fixed-and-locked scope on every monitor, rejected mismatched or unscoped actions, verified browser and QR bindings, blocked native plugins under scope, propagated rejected UIA batch actions, and validated unique decodable PNG captures.

| Run report | Report SHA-256 | Result |
|---|---|---|
| `monitor-smoke-20260715_003855_910_30964.json` | `405FBA46E1DC0476194C9A0ACB142290208C0124C74B26BE2BB022057982FCA6` | PASS, 13 of 13 |
| `monitor-smoke-20260715_003905_191_85748.json` | `B53E0C19FDF17AFCE2705BD426DE4A2B7C57FA47398ED136166990262CE401B5` | PASS, 13 of 13 |

The four physical stable IDs were identical across both runs. The IDs themselves are intentionally omitted from this public report.

## Rust and plugin checks

- Unit tests: 371 discovered; 370 passed, 0 failed, 1 ignored because the optional example plugin was not built.
- Clippy: all targets passed with warnings denied.
- Format check: passed.
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

The exact final installer copied the complete per-AI guide with its native Unicode clipboard path and logged the success popup. The acceptance harness restored the pre-test clipboard afterward. If native clipboard access is unavailable, the installer also tries original-user PowerShell fallbacks and truthfully points to the installed guide when every method fails.

## Release boundary

The artifacts are ready for signing and publication review, but this preparation did not sign, push, publish, edit live AI configuration, or restart a shared service.
