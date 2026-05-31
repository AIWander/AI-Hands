# AI-Hands Plugin ABI

This directory contains the public C-FFI plugin ABI for AI-Hands.

## Contents

- **`ai_hands_plugin.h`** — the authoritative C header. Plugin authors
  include this header and export the 5 entry points it declares. This
  file is the single source of truth for the ABI contract.
- **`example/`** — a standalone Rust crate demonstrating a minimal
  plugin. Builds independently of the AI-Hands workspace. See
  [`example/README.md`](./example/README.md) for build instructions.

The Rust mirror of these declarations lives in `src/plugin/abi.rs` inside
the host crate. Keep the two in lock-step.

## Why heavyweight C-FFI?

User decision recorded 2026-05-30 in the Milestone A handoff. The
short version:

> The C-FFI is the lowest-level interop boundary on every platform
> AI-Hands targets. Any higher-level binding (Lua, Python, JavaScript,
> Wasm) can be layered as a wrapper that exposes this ABI from the host
> side, without requiring changes to the core hands binary. Choosing
> C-FFI first keeps the door open for every other plugin model.
> Choosing Lua first would have locked plugins to that runtime.

## ABI contract overview

A plugin is a dynamic library (`.dll`/`.so`/`.dylib`) that exports five
C functions:

| Symbol | Purpose |
|---|---|
| `ai_hands_plugin_abi_version` | Returns the ABI version the plugin was compiled against. |
| `ai_hands_plugin_init` | One-time setup; returns plugin metadata + tool list. |
| `ai_hands_plugin_call` | Dispatches a tool call. May be invoked concurrently. |
| `ai_hands_plugin_free_string` | Frees a string the plugin allocated in `_call`. |
| `ai_hands_plugin_shutdown` | One-time teardown before unload. |

See [`ai_hands_plugin.h`](./ai_hands_plugin.h) for full signatures,
thread-safety expectations, and ownership rules.

## Versioning policy

The ABI version is a packed `uint32_t`: `(major << 16) | minor`.

- **Major bump** = breaking change. Struct layouts, enum values, or
  signature shapes change. Host **rejects** plugins compiled against a
  different major version.
- **Minor bump** = additive change only. New optional entry points, new
  status codes appended, new descriptor fields appended behind a length
  tag. Host **accepts** minor skew and may emit a warning. Plugins
  should compile against the lowest minor version they need.

Phase 1 ships ABI **v1.0**. Plugins built against this header today
will continue to work against any future v1.x host.

## Plugin location on disk

Once phase 2 lands, the host will auto-discover plugins from:

- **Windows**: `%LOCALAPPDATA%\hands\plugins\*.dll`
- **Linux**: `$XDG_DATA_HOME/hands/plugins/*.so`
  (or `~/.local/share/hands/plugins/*.so`)
- **macOS**: `~/Library/Application Support/hands/plugins/*.dylib`

You can also call `hands_plugin_load` with an absolute path to load a
specific binary outside the standard directories — useful during
development.

## Status

- **Phase 1 (current)**: ABI surface frozen. C header, Rust mirror,
  registry, and example plugin all shipped. The MCP tools
  `hands_plugin_list` and `hands_plugin_load` exist and respond
  correctly, but `hands_plugin_load` returns a clear "phase 2 pending"
  status instead of actually loading anything.
- **Phase 2 (future)**: add the `libloading` dep and wire the loader to
  actually `LoadLibrary`/`dlopen` the binary, verify the ABI version,
  call `ai_hands_plugin_init`, and register the plugin's tools into the
  MCP dispatch table.

The point of shipping phase 1 separately is that plugin authors can
start building today against a stable header without waiting on the
loader implementation.
