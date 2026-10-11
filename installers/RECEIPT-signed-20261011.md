# Receipt - signed AI-Hands 1.1.0-unified.2 release assets, 2026-10-11

Authority: Joseph to claude_chat, 18:35 EDT: "You can sign the hands release." Run by the claude_code delegate,
18:39-19:12 EDT, arm64 Surface, worktree C:\temp\wt-aihands-installer, branch installer/picker-20261011.
Scope: sign and refresh the DRAFT pre-release only. Not published, not merged, nothing pushed to main.

## Signer (PROVEN)

- Auth preflight 18:39: `az account show` = josephwander@gmail.com, subscription 50693697-...; a codesigning token
  was issued without interaction (expires 19:54). TrustedSigning module 0.5.8.
- Account AIWander, profile cpc-signing, via C:\CPC\scripts\sign-cpc.ps1. Certificate subject:
  `CN=Joseph Wander, O=Joseph Wander, L=Cooper City, S=fl, C=US`. Issuer: Microsoft ID Verified CS EOC CA 04.
  The subject names Joseph Wander, not AIWander. That is expected: this is an individual identity validation and
  SIGNING.md already says Windows shows "Joseph Wander". Cert valid 2026-10-09 to 2026-10-12 (a short-lived Trusted
  Signing cert); every signature is RFC3161-timestamped, so it stays valid after the cert expires.
- Timestamp authority: Microsoft Public RSA Time Stamping Authority (http://timestamp.acs.microsoft.com).

## Per-file signatures (PROVEN, Get-AuthenticodeSignature, 18:59-19:01)

| File | Status | Timestamp (UTC) | SHA-256 after signing |
|---|---|---|---|
| dist/HandsSetup-1.1.0-unified.2-win.exe | Valid | 2026-10-11 22:51:31 | 181dec79...712cdd9 |
| dist/payload/hands-arm64.exe | Valid | 22:41:30 | 88d7a4d9...41fabe |
| dist/payload/hands-x64.exe | Valid | 22:42:05 | 2256b3c1...cc952 |
| dist/payload/Register-HandsClients.ps1 | Valid | TSA countersigned | aad47e82...24e29 |
| portable zip: arm64/hands.exe, x64/hands.exe, Register-HandsClients.ps1 | Valid x3 | same as payload | same as payload |
| mcpb: server/hands.exe | Valid | 22:41:30 | 88d7a4d9...41fabe (= arm64 payload) |
| unins000.exe (from a test install) | Valid | TSA countersigned | 4effb83f...2f9f51 |

The uninstaller only exists after an install, so it was taken from a silent install to C:\temp\hands-sign-check
followed by a silent uninstall. Install exit 0, writer exit 2 (all three clients `skipped existing`), uninstall
exit 0, ARP key gone, folder gone. The three client configs were archived first to
`C:\CPC\backups\<file>.<Client>.pre_hands_sign_20261011_190059`. Their hash, mtime and backup count matched before
install, after install and after uninstall (ClaudeDesktop CAB13A25..., Codex 52C03A18..., GrokCli AB32845F...).
The live C:\CPC\servers\hands.exe (PID 23028) kept running throughout.
Smoke test of the signed arm64 and x64 binaries: MCP initialize returns serverInfo hands 1.1.0-unified.2, and
tools/list returns 105.

## Hashes before and after signing

| Asset | Unsigned (04:33, RECEIPT-dist) | Signed (18:51-18:57) |
|---|---|---|
| HandsSetup-1.1.0-unified.2-win.exe | f1489496...a5d42 (11839103 B) | 181dec79063cd111c3a7f20c9ed457af9064d8f2cff633650fe7d071c712cdd9 (11892536 B) |
| HandsSetup-portable.zip | 48cec90b...c5ffa | 8afe7ffbb1e5015dc87b6027532258b2d26dbb6b5ebda5af2b9f223996043833 |
| hands-1.1.0-unified.2-win-arm64.mcpb | 8c509287...763ad | 6cb9792a7a92416ffaf7d7cf1d6fcd296d6db08651219fb16259c4e70368a0b1 |
| payload/hands-arm64.exe | d2ebeb4a...515cb | 88d7a4d9e5d326c2571e11dd37d551e18b4b761d43338b2711402f001041fabe |
| payload/hands-x64.exe | e6dd240d...66d7cf2 | 2256b3c1ecfac72181071dce18f289e46f5b696f97a0dc0c18365edf079cc952 |
| payload/Register-HandsClients.ps1 | fe45f85a...8eae | aad47e82ae23d20a0f43ee1aff71f517b300e468682a1ff74f203762d7924e29 |

`sha256sum -c dist/SHA256SUMS.txt`: all 7 OK. Build log: dist/build-sign.log (not committed; dist is untracked).

## Manifests (PROVEN)

`update-manifests.ps1` rewrote the winget installer.yaml (InstallerSha256 181DEC79... for x64 and arm64) and
scoop/ai-hands.json (hash 8afe7ffb... for 64bit and arm64). The other two winget files did not change.
`update-manifests.ps1 -Check` exit 0. `winget validate` on the 1.1.0-unified.2 folder: "Manifest validation
succeeded." The Scoop JSON parses.

## Commits and release state (PROVEN, 19:05-19:10)

- 5d35f53, "chore(installers): signed 1.1.0-unified.2 release assets; manifests updated with signed hashes",
  pushed to installer/picker-20261011; draft PR #23 follows the branch. The first push was rejected because
  a52c89f (AIWander, 18:47, "style: cargo fmt build.rs") had landed after my 18:39 pull, so I rebased my one
  unpushed commit onto it. That diff only re-wraps lines (same tokens and literals). The signed binaries were built
  from the pre-fmt build.rs, and that has no effect on the binaries (INFERRED from the token-identical diff).
- Release v1.1.0-unified.2: isDraft=true, isPrerelease=true, no remote tag (ls-remote empty), the only draft.
  The 4 assets were replaced with --clobber. All 4 GitHub digests match dist/SHA256SUMS.txt (SHA256SUMS.txt
  itself is c2eaed7a...). Notes: only line 1 changed, from the UNSIGNED DRAFT warning to "Signed with AIWander's
  code-signing certificate (Azure Trusted Signing); SHA256SUMS.txt lists the signed artifacts." The draft URL
  slug changed on edit (now untagged-de60b83ce63ebf173f03). The release TITLE still reads
  "AI-Hands 1.1.0-unified.2 (unsigned draft)". I left it unchanged because the packet allowed only the notes line.

## Still Joseph's decisions

1. Publish the draft. Publishing creates tag v1.1.0-unified.2, and that starts .github/workflows/release.yml.
2. Canonical downloads: the CI tag build attaches its own (unsigned) artifacts. Choose these signed assets or the
   CI ones, or have CI skip uploading assets.
3. Release title: drop "(unsigned draft)" before publishing.
4. AIHands.iss (A2D39077) vs the picker installer (6E0B2C47): coexistence or retirement.
5. Silent-install default: a silent or winget install ticks every detected client.
6. x64 .mcpb: only arm64 is packed today.
7. Optional cleanup: C:\CPC\backups\*.pre_hands_sign_20261011_190059 (3 files) and *.pre_hands_e2e_20261011_*.

## Proven / inferred / untested

- PROVEN: auth preflight; all signatures above (Valid, timestamped); uninstaller signed; configs byte-identical;
  signed binaries serve MCP (105 tools); sums, manifests, winget validate; push; draft state and asset digests.
- INFERRED: SmartScreen reputation for the new cert/publisher (signing does not guarantee no prompt); the
  build.rs fmt commit has no effect on the binaries; the x64 install path (host is arm64).
- UNTESTED: install on another machine; real winget/scoop install (needs a published release and tag); the
  interactive wizard on the signed build (only /SILENT was run).
- Deviation: the autonomous gateway returned "gateway busy" twice on breadcrumb_start, so this run has no CPC
  breadcrumb. The commit carries the Fable 5.1 trailer as the packet asked; the session ran on Opus 5.5.
