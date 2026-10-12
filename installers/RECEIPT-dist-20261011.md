# Receipt - Hands picker installer to distribution grade (packet B-DIST), 2026-10-11

Branch `installer/picker-20261011` (worktree C:\temp\wt-aihands-installer). Work commit 48b39dc on 367a7fa;
this receipt is the next commit. Run clock 04:18-04:37 EDT, arm64 Surface, Windows 11 Home 26200.
Draft PR: https://github.com/AIWander/AI-Hands/pull/23 (draft, open, not merged)
Draft pre-release: https://github.com/AIWander/AI-Hands/releases/tag/untagged-1aa6d8def2bdc127daf8
(tag name v1.1.0-unified.2, isDraft=true, isPrerelease=true; `git ls-remote --tags` shows no such tag yet)

## Gap audit (before -> after)

| Item | Before (367a7fa) | After (48b39dc) |
|---|---|---|
| AppId | 6E0B2C47..., no comment | same GUID kept (unreleased, generated once), "do not change" comment |
| Version resources | none on setup or hands.exe | setup 1.1.0.2 + text 1.1.0-unified.2; hands.exe FileVersion 1.1.0.2, CompanyName AIWander (build.rs) |
| Icon | Inno default | original hand mark: installers/assets (svg + generator, ico, png); setup, uninstall entry, hands.exe |
| License / privacy | none | LicenseFile=..\LICENSE (Apache-2.0); InfoAfterFile=..\PRIVACY.md; PRIVACY.md installed |
| Welcome | default, DisablePage default | DisableWelcomePage=no, one-paragraph WelcomeLabel2 (corrected, see deviation 1) |
| Output | HandsSetup-<ver>.exe, lzma2 | HandsSetup-<ver>-win.exe, lzma2/ultra64, solid |
| Privileges | lowest only | lowest + PrivilegesRequiredOverridesAllowed=dialog; all-users goes to {autopf}; writer runasoriginaluser |
| Running hands.exe | not handled | CloseApplications=yes, filter hands.exe, no restart |
| Upgrade choice | detection re-ticked everything | previous choice stored and reused exactly; explicit /COMPONENTS wins |
| Uninstall leftovers | register.log only | + dirifempty {app}; finish page states backups folders are kept |
| Signing | post-sign setup only | payload signed pre-pack + Inno SignTool (uninstaller and setup); SIGNING.md |
| Docs | README-picker (tech notes) | end-user guide, PRIVACY.md, SIGNING.md, CHANGELOG entry, root README links |
| Manifests | winget/scoop 1.0.1 only | winget 1.1.0-unified.2 (inno) + scoop 1.1.0-unified.2 (zip), update-manifests.ps1 [-Check] |

## Files changed (19)

CHANGELOG.md, Cargo.toml (+[build-dependencies] winresource), Cargo.lock, README.md, build.rs (new, 30 lines),
PRIVACY.md (new), installers/HandsSetup.iss, installers/README-picker.md, installers/build-hands-setup.ps1,
installers/SIGNING.md (new), installers/update-manifests.ps1 (new), installers/assets/{hands-icon.svg,.ico,.png,
make-icon.py} (new), installers/scoop/ai-hands.json, installers/winget/manifests/.../1.1.0-unified.2/*.yaml (3 new).
Outside the worktree: config archives only (see e2e); scratch harness in C:\temp\hands-e2e-harness.

## Build evidence (PROVEN, 04:27-04:33)

`build-hands-setup.ps1 -Portable`: arm64 and x64 compiled with build.rs. The first ISCC run failed on
`UsePreviousComponents` (not an Inno 6 directive; removed, since UsePreviousSetupType=no covers it), then
`-Portable -SkipBuild` exit 0. Final dist/SHA256SUMS.txt (`sha256sum -c`: all OK):

```
f148949624c2690f9c6faddec61979a0c2d67d58d33429d21aa0f406e33a5d42  HandsSetup-1.1.0-unified.2-win.exe (11.3 MB)
48cec90b65a815e5950c774dabfcdfc946035f828cae58e236310b4dc21c5ffa  HandsSetup-portable.zip
8c509287b522dabe64bd64c715dc0f8d9237a4107c2adc7c8abf5f60d27763ad  hands-1.1.0-unified.2-win-arm64.mcpb
d2ebeb4a630602af5a0d7b9867264b1e77b61e7665e5d37bb0e32c208fd515cb  payload/hands-arm64.exe
e6dd240da17a609ae2304c8e9b3e37eedebb6ed5b8c7a967ee562ca1e66d7cf2  payload/hands-x64.exe
fe45f85a9902178b643da0f4f582321d3d16b2b5f134d0aa0679dce21f568eae  payload/Register-HandsClients.ps1 (unchanged)
```

- Both binaries: MCP initialize -> serverInfo hands 1.1.0-unified.2, tools/list = 105. All four exes NotSigned.
- SignTool hook: ISCC with /DUseSignTool=1 and a dummy `cpcsign` ran it on `uninst.e32.tmp`, then refused it:
  "returned an exit code of 0, but the file does not have a digital signature".
- `winget validate` on the 1.1.0-unified.2 folder: "Manifest validation succeeded" (after Documentation ->
  Documentations). Scoop JSON parses. `update-manifests.ps1 -Check` exit 0. Release asset digests on GitHub
  match the sums above.

## E2E evidence (PROVEN, 04:33:47-04:34:13; first full run 04:31 also passed)

Windows Sandbox: WindowsSandbox.exe absent, and Get-WindowsOptionalFeature needs elevation (Windows 11 Home).
Ran locally with /DIR=C:\temp\hands-e2e\app. The configs were archived first to
`C:\CPC\backups\<file>.<Client>.pre_hands_e2e_20261011_043126` and `..._043347`.
Baseline, identical after every phase (hash, mtime, and 0 `*.pre_hands_*` backups):

```
ClaudeDesktop 08DE327AC494AE18FBAA8728DA9D984B85791166CBB146BCA6E7014CA3DC7F37 mtime 2026-10-11T00:29:47
Codex         1D0B4442F95D55163A990C19407ED5C86744C00B6A99E9348D4AD85FE96D3150 mtime 2026-10-11T00:29:46
GrokCli       AB32845F0D1696AA168138D38EC53812337F6B86CFFF30403A4AF781DF38A16A mtime 2026-10-04T03:02:26
```

- A install: exit 0; files hands.exe, ps1, README, PRIVACY, icon, register.log, unins000.*; register.log
  `ClaudeDesktop/Codex/GrokCli: skipped existing`; Inno log `Process exit code: 2`; HKCU ARP key
  `{6E0B2C47-...}_is1` DisplayName 'AI-Hands 1.1.0-unified.2', Publisher AIWander, DisplayIcon hands-icon.ico.
- B uninstall: writer `-Uninstall` exit 2 (all three entries are foreign); `Deleting registry key ... _is1`;
  every file deleted; app folder and its parent deleted. ARP absent, nothing left on disk.
- C upgrade: unified.1 setup with /COMPONENTS=codex -> Clients='Codex'; release setup over it with no switch:
  `Will append to existing uninstall log`, `Deleting uninstall key left over from previous ... install`,
  DisplayVersion moved to unified.2, writer ran for Codex only. A same-version re-run was clean.
- D final uninstall: ARP absent, folder gone, configs unchanged. Harness exit 0, "E2E PASS".
- The first run caught a bug: /COMPONENTS was overridden by InitializeWizard. Fixed (ComponentsOnCommandLine),
  rebuilt, and the full re-run passed.

## Deviations

1. Welcome text: the packet's "local-only" claim is not literally true. vision_analyze uploads a screenshot to
   api.anthropic.com when ANTHROPIC_API_KEY is set, and an opt-in loopback dashboard binds 127.0.0.1:9102
   (HANDS_ENABLE_DASHBOARD=1). The wording says "runs on this PC, no telemetry or update check, online only when
   your AI asks", and PRIVACY.md discloses both.
2. Kept the existing picker AppId instead of a new GUID: unreleased, generated once tonight. AIHands.iss
   (A2D39077) stays separate because its layout differs.
3. Signing goes through Inno's SignTool directive instead of post-signing the setup, so the uninstaller is
   signed too. It uses the same sign-cpc.ps1 and profile as build-cpc-installer.ps1.
4. Added PrivilegesRequiredOverridesAllowed handling: DefaultInstallDir uses {autopf} for all-users installs,
   and [Run] runs as the original user.
5. Autonomous breadcrumb_start failed ("durable session capacity reached"), so this run has no CPC breadcrumb.
6. The commit carries the Fable 5.1 trailer the packet asked for; the session ran on Opus 5.5.

## Proven / inferred / untested

- PROVEN: everything under Build and E2E above; VERSIONINFO read back from all three exes; manifests valid.
- INFERRED: the x64 install path (same [Files] logic with Check: not IsArm64; this host is arm64); the
  all-users mode and runasoriginaluser (compiled, not run elevated); CloseApplications against a running
  hands.exe (RmGetList ran, no holder in the test folder); the interactive wizard pages (license, privacy,
  picker, finish text) render (compiled, run only /SILENT).
- UNTESTED: signing (owner identity); a clean machine or VM (no Sandbox on Home); a real winget/scoop install
  (needs a published release); first-time writes to a client without a hands entry on this machine (covered
  only by the 60/60 writer fixtures from the earlier receipt; writer file unchanged, hash fe45f85a...).

## Remaining human steps (Joseph)

1. Review draft PR #23.
2. Sign: `pwsh -NoProfile -File installers\build-hands-setup.ps1 -Portable -Sign`, verify with
   Get-AuthenticodeSignature, then `installers\update-manifests.ps1`, and commit the manifests (SIGNING.md).
3. `gh release upload v1.1.0-unified.2 <4 assets> --clobber`, and drop "unsigned draft" from the notes.
4. Decide open questions: canonical assets vs .github/workflows/release.yml (publishing creates the tag that
   starts it); AIHands.iss vs picker coexistence; silent-install default ticks detected clients; x64 .mcpb.
5. Publish the release, merge the PR, then submit winget (microsoft/winget-pkgs) and keep the Scoop manifest
   in-repo.
6. Optional: delete the six e2e config archives in C:\CPC\backups (*.pre_hands_e2e_20261011_*).
