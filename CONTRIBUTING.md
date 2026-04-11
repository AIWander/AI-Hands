# Contributing to Hands

## Filing Issues

Open an issue on [GitHub](https://github.com/josephwander-arch/hands/issues). Include:

- What you were trying to do
- What happened instead
- Your platform (x64 or ARM64, Windows version)
- The tool(s) involved
- Relevant error output

Don't paste full debug logs in the issue body — attach them as a file or use a collapsible section.

## Pull Requests

1. Fork the repo and create a feature branch from `main`.
2. Make your changes. Keep commits focused — one logical change per commit.
3. Test on at least one platform (x64 or ARM64 Windows).
4. Open a PR against `main` with a clear description of what and why.

PRs that touch tool behavior should include a before/after example showing the change.

## Build Instructions

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- Windows 10/11 (UIA tier requires Windows APIs)

### x64 Build

```bash
cargo build --release --target x86_64-pc-windows-msvc
```

Output: `target/x86_64-pc-windows-msvc/release/hands.exe`

### ARM64 Build

```bash
cargo build --release --target aarch64-pc-windows-msvc
```

Output: `target/aarch64-pc-windows-msvc/release/hands.exe`

### Cross-compile Notes

ARM64 builds require the ARM64 MSVC toolchain component. Install via Visual Studio Installer or:

```bash
rustup target add aarch64-pc-windows-msvc
```

## Code Style

- Rust stable, no nightly features.
- `cargo fmt` before committing.
- `cargo clippy` should pass clean.
- Keep tool implementations self-contained where possible — each tool should be understandable without reading the entire codebase.

## Questions

Email protipsinc@gmail.com or open a discussion on GitHub.
