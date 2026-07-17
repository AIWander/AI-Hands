# AI-Hands v1.1.0-unified.2 validation

This report identifies the exact ARM64 and x64 artifacts prepared for source review after the fixed-monitor identity repair. Both standalone Rust executables, the `hand.exe` ARM64 template alias, and all eight self-installing plugin variants are Authenticode signed and timestamped. None has been released from this branch.

## Final Rust binaries

| File | Architecture | Signed size | Final signed SHA-256 |
|---|---:|---:|---|
| `dist/AI-Hands-v1.1.0-unified.2-arm64.exe` | ARM64 | 22,009,616 bytes | `B23F869392A3094808DDA2C55B5A9DFC1F21B539AE30ECC14BF39125004D2A9B` |
| `dist/AI-Hands-v1.1.0-unified.2-x64.exe` | x64 | 26,665,232 bytes | `285898F18E92B997B1E85F15D22748E1BDC16221CC03B08B97A71D8FB6BBE31F` |
| `dist/hand.exe` | ARM64 template alias | 22,009,616 bytes | `B23F869392A3094808DDA2C55B5A9DFC1F21B539AE30ECC14BF39125004D2A9B` |

- Signature state: all three files report Valid and carry an RFC 3161 timestamp from the publisher certificate.
- Embedded-string checks: no workstation-specific username path, private user path, old live-binary hash, private coverage filename, hardcoded Node executable, or private JavaScript-helper path. Generic, non-user-specific fallback strings from the public path-discovery dependency remain by design.
- Dependency check: one vendored `vision-core` package is shared by Hands and the vendored browser library.
- Fixed-monitor identity contract: Windows device-interface paths are normalized and converted inside Rust to deterministic opaque `physical:v1:<sha256>` tokens before tool-output redaction. `hands_monitor_scope action=list` now returns a token that can be fed back into `HANDS_MONITOR_SCOPE=stable:<token>` without exposing or destroying the underlying identity.

## Current monitor acceptance

Both exact signed binaries above passed the strict monitor harness on the one primary monitor currently connected. These results are diagnostic coverage rather than four-monitor acceptance. ARM64 enumerated the monitor as a physical display and proved the opaque fixed-token round trip across processes. The x64 executable, running through Windows-on-ARM emulation, enumerated the same display as a logical primary and therefore proved the locked primary fallback instead of fixed physical binding. Each run still exercised every connected monitor and proved the applicable binding mode, locked-without-scope refusal, mismatch refusal, locked-scope mutation refusal, browser and QR binding, native-plugin refusal, aggregated UIA batch failure, persistent route and trace refusal, six collision-free same-process captures, four collision-free concurrent-process captures, valid PNG decode and dimensions, and cleanup of generated captures.

| Run report | Report SHA-256 | Result |
|---|---|---|
| `monitor-smoke-20260717_001905_911_163980.json` | `9F80735B57E63C2316A3037ACDEBF9812F17F49EE4FA55F2A0D26DD0223A455C` | ARM64 `B23F8693...`; physical fixed binding; `PASS_DIAGNOSTIC`; 17/17 true |
| `monitor-smoke-20260717_002040_917_64236.json` | `02B4325AFF8F0A3929E046F6B8181246AE44F1B7067C011220C55F7FD6AD7909` | x64 `285898F1...`; emulated logical-primary fallback; `PASS_DIAGNOSTIC`; 17/17 true |

The earlier `.2` candidate failed because general output redaction replaced the returned Windows device path with literal `[PRIVATE_PATH]`, so a second process could not bind to the advertised ID. The new ARM64 regression diagnostic proved the advertised and re-enumerated opaque IDs match across processes, and the exact signed artifacts pass their available architecture lanes. An earlier signed `.1` build passed the four-physical-monitor suite twice, but that evidence is not substituted for an exact-binary four-monitor claim on these final `.2` artifacts. Four-monitor acceptance must be rerun when the additional displays are connected, and physical fixed binding for the exact x64 artifact must be rerun on native x64 Windows rather than inferred from emulation.

## Rust, security, and plugin checks

- Unit tests: 415 discovered; 414 passed, 0 failed, 1 ignored because the optional example plugin was not built.
- Clippy: all targets and features passed with warnings denied.
- Format check: passed.
- Browser crate tests: 5 passed.
- Plugin-profile tests: 9 passed.
- Shared lifecycle and safety-policy tests: 6 passed.
- RustSec audit: exit 0 for the Windows targets; remaining warnings are unmaintained `fxhash` and `rustls-pemfile`, plus a `memmap2` warning reached through the Linux-only screenshot/Wayland path.
- Cargo Deny: the CI-scoped `licenses bans sources` check exits 0; remaining diagnostics are warnings, not denied findings. A full all-platform advisory check also sees `quick-xml` advisories through Linux-only Wayland metadata, but AI-Hands is Windows-only and the released-Windows RustSec lane does not compile that path.
- Codex plugin validator: passed.
- Four bundled AIHands/Grok skill validators: passed.
- JSON package registrations: parsed successfully.
- Profile smoke tests: default 105 listed, full 107, strict 109, compatibility 144; every list was unique, grouped, and contract-consistent.
- Headless browser security smoke: passed. A page-injected performance row was reduced to the closed safe schema; hostile `headers`, `body`, `extra`, and sentinel values did not escape.
- Final-output path smoke: only existing Hands-generated captures under the canonical capture roots can retain a path, and only from the narrow capture-tool/key allowlist; page-provided and arbitrary paths remain redacted.
- Signed Voice payload smoke: both pinned v0.3.1 Rust wrappers reported the expected server identity and all 10 tools.
- Signed installer cold tests: 10 scenarios passed across ARM64 and x64: hooks/no-Voice, hooks/Voice, skills/no-Voice, skills/Voice, and a hooks+Voice to skills/no-Voice transition on each architecture. Every scenario verified the embedded payload hash, selected plugin identity, optional Voice presence or absence, marketplace registration, instruction artifacts, valid installer signature, and clean uninstall.

## Windows self-installing plugin matrix

| Profile | Voice | ARM64 SHA-256 | x64 SHA-256 |
|---|---|---|---|
| hooks | no | `9028B2C70F61BA5F71632DA57F94D8719BC49DAD03F7ECFAC40673E2405FEB45` | `29779FD8AAB17CE3C47A014D0DD0375D6F2B42FEAC89BBA79ACDB0EC2F13731E` |
| hooks | yes | `9DBC516C40C66B828BE07B6FBD096068D793EDBF13B9896C357FF2703187A9B0` | `8CEBC712CF8463C13A2CBA6B0DED4F02CD56C0421E995E7D30A2C611CA86782D` |
| skills | no | `7CF8E4654478ADD2335013C96DA5064C2FC7210B6D9E3774FFAEB771FBF516E8` | `D5ECB622B0BBC4535692BDA2AC56F769E133E46680FCA2A3094D9EDDBFEBEE62` |
| skills | yes | `45AB09086624E7A28D0BFC1687E7DA9494D84937EB24A57EE6714B8342002724` | `1F1E9D29CF88E3E8D067231F07447511E3329FB03FE3026040F258FEE3CD18E2` |

- Signature state: all eight installers report Valid, are timestamped, and have unique hashes.
- Hooks profile: four purpose-grouped AIHands skills plus opt-in hook adapters and shared policy; it does not silently enable or trust host hooks.
- Skills profile: the same four purpose-grouped skills without hook files or duplicate tool coverage.
- Voice remains a separate optional companion in both profiles.
- Interactive install copies the universal host-instruction addition to the clipboard and reports how to apply it; silent install leaves result and instruction artifacts instead of pretending a UI action occurred.
- The local `dist` directory contains no generic pre-matrix installer. The required `hand.exe` template alias is byte-identical to the named ARM64 binary.

## Release boundary

The source and local signed staging matrix are ready for pull-request review. A GitHub release tag is not ready: the repository currently has zero Actions signing secrets, so the release workflow correctly fails closed before upload. The local Inno Setup 6.7.0 compiler also reports `Non-commercial use only`; the publisher's current [commercial-license guidance](https://jrsoftware.org/isorder.php) requests licenses from commercial users while stating purchase is not strictly required. Record Joseph's production/compliance choice before distributing these Inno-built installers commercially.

Four-monitor exact-artifact acceptance remains a live-promotion gate, not a source-merge gate. Repository branch governance is also a separate owner decision: main currently has neither classic branch protection nor a repository ruleset. No live Hands binary or AI configuration was replaced, no plugin or hook was activated, no tunnel was changed, and no shared service was restarted.
