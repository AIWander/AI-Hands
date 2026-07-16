# AI-Hands v1.1.0-unified.2 validation

This report identifies the exact ARM64 publication artifacts prepared for source review. The standalone Rust executable and the self-installing Windows plugin package are Authenticode signed; neither artifact has been released from this branch.

## Final Rust binary

- File: `dist/hand.exe`
- Final signed SHA-256: `5A4A09429C0B228753AF1B50EFD983DAD06C26FAD756C8F98D63F9BDDE136F93`
- Final signed size: 21,989,136 bytes
- PE architecture: ARM64 (`0xAA64`)
- Signature state: Valid and timestamped with the publisher certificate
- Embedded-string checks: no workstation-specific username path, private user path, old live-binary hash, private coverage filename, hardcoded Node executable, or private JavaScript-helper path. Generic, non-user-specific fallback strings from the public path-discovery dependency remain by design.
- Dependency check: one vendored `vision-core` package is shared by Hands and the vendored browser library.

## Current monitor acceptance

The exact final signed binary above passed the strict monitor harness twice. Only one logical primary monitor was connected for these runs, so the result is diagnostic coverage rather than four-monitor acceptance. Both runs proved fail-closed scope policy, one virtual-primary capture, six collision-free same-process captures, four collision-free concurrent captures, valid PNG output, scope-mutation rejection, and the policy probes available on the current topology.

| Run report | Report SHA-256 | Result |
|---|---|---|
| `monitor-smoke-20260716_143159_061_75296.json` | `737619AFB884546F72557C69953D4D9FF31CBEFF5380F67387D3B58041861796` | `PASS_DIAGNOSTIC`; one connected logical primary, every test true |
| `monitor-smoke-20260716_143201_149_73440.json` | `86705E78ED0300997D8397357879225458B605F567B76EFF8F633FF49E58918B` | `PASS_DIAGNOSTIC`; repeat run, same topology, every test true |

An earlier signed `.1` build passed the four-physical-monitor suite twice. That is useful regression evidence for the monitor design, but it is not substituted for an exact-binary four-monitor claim on this `.2` artifact. Fixed-physical unattended acceptance must be rerun when multiple physical monitors are connected.

## Rust, security, and plugin checks

- Unit tests: 414 discovered; 413 passed, 0 failed, 1 ignored because the optional example plugin was not built.
- Clippy: all targets and features passed with warnings denied.
- Format check: passed.
- Browser crate tests: 5 passed.
- Hook/plugin policy tests: 5 passed.
- RustSec audit: exit 0 for the Windows targets; remaining warnings are unmaintained `fxhash` and `rustls-pemfile`, plus a `memmap2` warning reached through the Linux-only screenshot/Wayland path.
- Cargo Deny: exit 0; remaining diagnostics are warnings, not denied findings.
- Codex plugin validator: passed.
- Four bundled AIHands/Grok skill validators: passed.
- JSON package registrations: parsed successfully.
- Profile smoke tests: default 105 listed, full 107, strict 109, compatibility 144; every list was unique, grouped, and contract-consistent.
- Headless browser security smoke: passed. A page-injected performance row was reduced to the closed safe schema; hostile `headers`, `body`, `extra`, and sentinel values did not escape.
- Final-output path smoke: only existing Hands-generated captures under the canonical capture roots can retain a path, and only from the narrow capture-tool/key allowlist; page-provided and arbitrary paths remain redacted.

## Windows self-installing plugin

- File: `dist/AIHands-Setup-1.1.0-unified.2-arm64.exe`
- SHA-256: `74A7C9B827597C8D253969DEBBB05B77869602ABC81DD9D033845187DD8A9B0E`
- Size: 7,199,088 bytes
- PE architecture: ARM64 (`0xAA64`)
- Signature state: Valid and timestamped with the publisher certificate
- Contents: signed Rust server, Codex and Claude plugin manifests, MCP registration template, two AIHands skills, host-neutral instructions, and opt-in hook fragments.
- Default behavior: does not edit AI host configuration, enable hooks, start a server, alter a tunnel, or add PATH unless that selectable task is enabled.
- Interactive behavior: attempts to copy the universal application guide and reports whether clipboard setup succeeded; silent mode leaves activation and result files instead of claiming an interactive action occurred.

A prior isolated installer cycle verified the payload layout, absolute MCP command, disabled PATH task, no host/hook/server mutation, and clean uninstall, but it used an earlier signed `.2` binary. The final installer was deliberately not rerun on the same Windows account because Inno's fixed application identity could replace the existing per-user uninstall registration even with a custom directory. The release workflow now performs a cold install/hash/config/uninstall gate on each fresh x64 and ARM64 runner before artifact upload; that gate must be green before release.

## Release boundary

The source, signed Rust binary, and signed installer are ready for pull-request review. Publication requires green CI, including the fresh-runner self-installing-package gate. No live Hands binary or configuration was replaced, no hook was enabled, no tunnel was changed, and no shared service was restarted. Inno Setup's base license permits commercial use; its vendor requests commercial users purchase a license but states that purchase is not strictly required. Record the publisher's support and compliance choice before commercial distribution; it is not a technical build blocker. See the [official commercial-license guidance](https://jrsoftware.org/isorder.php).
