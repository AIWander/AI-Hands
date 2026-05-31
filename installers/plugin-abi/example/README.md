# Example AI-Hands plugin

A minimal Rust plugin demonstrating the AI-Hands C-FFI plugin ABI.

## Important

This crate is **NOT** a member of the AI-Hands workspace. It is a
**standalone Rust package** that lives inside the AI-Hands repository for
discoverability, but is built independently.

That means:

- `cargo build` from the repo root will **not** build it.
- Running `cargo build` from this directory will.
- It pulls no third-party dependencies — only `std` — so a plugin author
  can read the source top-to-bottom and see exactly what the ABI requires.

## What it does

The plugin exposes a single tool: `example_echo`. It takes
`{ "message": "<text>" }` and returns `{ "echoed": "<text>" }`. That's
it. The point isn't what it does — the point is that it shows the five
ABI entry points (`ai_hands_plugin_abi_version`, `ai_hands_plugin_init`,
`ai_hands_plugin_call`, `ai_hands_plugin_free_string`,
`ai_hands_plugin_shutdown`) wired up correctly.

## Build

```bash
cd installers/plugin-abi/example
cargo build --release
```

The output binary lives at:

- Windows: `target/release/example_plugin.dll`
- Linux:   `target/release/libexample_plugin.so`
- macOS:   `target/release/libexample_plugin.dylib`

## Load (phase 2)

Plugin loading wiring lands in phase 2 of Milestone A Item 7. Once it's
in, you'll load this plugin by either:

1. Dropping the binary into the host's plugin directory:
   - Windows: `%LOCALAPPDATA%\hands\plugins\`
   - Linux: `$XDG_DATA_HOME/hands/plugins/` (or `~/.local/share/hands/plugins/`)
   - macOS: `~/Library/Application Support/hands/plugins/`
2. Calling `hands_plugin_load` with an absolute path to the binary.

In phase 1 (current), `hands_plugin_load` validates the path exists and
returns a clear "phase 2 wiring pending" status instead of attempting to
load. The C ABI is **frozen** in phase 1, so plugins built today against
`installers/plugin-abi/ai_hands_plugin.h` will continue to work once
phase 2 ships.

## Use as a template

Copy this directory somewhere outside the AI-Hands repo, rename the crate
in `Cargo.toml`, replace the body of `ai_hands_plugin_call` with your
real logic, and build. The header file in
`installers/plugin-abi/ai_hands_plugin.h` is the authoritative ABI
contract — read it before adding more tools.
