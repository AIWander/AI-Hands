// Embeds Windows VERSIONINFO and the AI-Hands icon into hands.exe (Explorer, Apps & Features, SmartScreen).
// The numeric version is major.minor.patch.N, N being the trailing number of a pre-release tag
// (1.1.0-unified.2 -> 1.1.0.2), the same rule installers/build-hands-setup.ps1 uses for the installer.
fn main() {
    println!("cargo:rerun-if-changed=installers/assets/hands-icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ver = env!("CARGO_PKG_VERSION");
    let (core, pre) = ver.split_once('-').unwrap_or((ver, ""));
    let mut n: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    n.resize(3, 0);
    n.push(pre.rsplit('.').next().and_then(|p| p.parse().ok()).unwrap_or(0));
    let packed = (n[0] << 48) | (n[1] << 32) | (n[2] << 16) | n[3];
    let mut res = winresource::WindowsResource::new();
    res.set_icon("installers/assets/hands-icon.ico")
        .set("CompanyName", "AIWander")
        .set("ProductName", "AI-Hands")
        .set("FileDescription", "AI-Hands MCP server: browser, Windows UI Automation and screen/OCR tools")
        .set("LegalCopyright", "Copyright 2026 Joseph Wander / AIWander. Apache-2.0.")
        .set("OriginalFilename", "hands.exe")
        .set("FileVersion", &format!("{}.{}.{}.{}", n[0], n[1], n[2], n[3]))
        .set("ProductVersion", ver)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
    if let Err(e) = res.compile() {
        panic!("embedding VERSIONINFO failed: {e}");
    }
}
