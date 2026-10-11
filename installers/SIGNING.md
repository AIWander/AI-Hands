# Signing the AI-Hands installer

Release builds are signed with AIWander's Azure Trusted Signing profile, the same one every CPC installer
uses (`C:\CPC\scripts\sign-cpc.ps1`, account `AIWander`, profile `cpc-signing`; the publisher shown by
Windows is "Joseph Wander"). Signing runs under that identity, so only its owner runs it. Automated builds
and agents do not.

## What `-Sign` signs, in order

`installers\build-hands-setup.ps1 -Sign`:

1. builds `hands.exe` for arm64 and x64 and stages them as `dist\payload\hands-arm64.exe` and
   `hands-x64.exe`;
2. signs both binaries and `Register-HandsClients.ps1` in `dist\payload` with `sign-cpc.ps1`, before
   anything is packed, so the installer, the portable zip and the `.mcpb` all carry signed files;
3. runs ISCC with `/DUseSignTool=1 "/Scpcsign=pwsh -NoProfile -File C:\CPC\scripts\sign-cpc.ps1 -Path $f"`.
   `HandsSetup.iss` then sets `SignTool=cpcsign` and `SignedUninstaller=yes`, so Inno signs the embedded
   uninstaller (`unins000.exe`) and the finished `HandsSetup-<version>-win.exe`;
4. builds the portable zip and the `.mcpb` from the signed payload and writes `dist\SHA256SUMS.txt`.

This matches `C:\CPC\installers\shared\build-cpc-installer.ps1`: same script and profile. The one
difference is that the setup exe is signed through Inno's SignTool hook rather than afterwards, so the
uninstaller is signed too.

## Commands to run (owner, on the build machine)

```powershell
# one-time: az login (or the AZURE_CLIENT_ID / AZURE_TENANT_ID / AZURE_CLIENT_SECRET service principal),
#           Install-Module TrustedSigning -Scope CurrentUser
cd C:\temp\wt-aihands-installer          # or any checkout of the release commit
pwsh -NoProfile -File installers\build-hands-setup.ps1 -Portable -Sign

# verify: every line should say Valid, signer Joseph Wander
Get-ChildItem dist\payload\*.exe, dist\payload\*.ps1, dist\HandsSetup-*-win.exe |
    Get-AuthenticodeSignature | Format-Table Status, @{n='Signer';e={$_.SignerCertificate.Subject}}, Path -AutoSize

# signing changed every hash: refresh the manifests, then commit them
pwsh -NoProfile -File installers\update-manifests.ps1
git add installers\winget installers\scoop
git commit -m "chore(installers): manifest hashes for the signed build"
```

Then replace the assets on the draft release (it was created unsigned) and publish:

```powershell
$v = '1.1.0-unified.2'
gh release upload "v$v" "dist\HandsSetup-$v-win.exe" dist\HandsSetup-portable.zip "dist\hands-$v-win-arm64.mcpb" dist\SHA256SUMS.txt --clobber -R AIWander/AI-Hands
```

## Notes

- `update-manifests.ps1` must run again after every signing run or rebuild. The winget and Scoop hashes
  come from `dist\SHA256SUMS.txt`, and signing rewrites the files. `update-manifests.ps1 -Check` exits 1
  when the committed manifests do not match the sums.
- Publishing the release creates the tag `v<version>`, and that tag push starts
  `.github/workflows/release.yml`, which builds and attaches its own artifacts. Decide whether those or
  these locally built installer assets are the canonical downloads before publishing.
- If signing fails partway, nothing is published. Rerun the whole command; the build starts from a clean
  `dist\payload`.
