/*
 * ai_hands_plugin.h — AI-Hands C-FFI plugin ABI (heavyweight path)
 *
 * ============================================================================
 * OVERVIEW
 * ============================================================================
 *
 * This header defines the binary contract between the AI-Hands host
 * (hands.exe) and third-party plugins distributed as dynamic libraries
 * (.dll on Windows, .so on Linux, .dylib on macOS).
 *
 * The host loads a plugin by:
 *   1. dlopen()/LoadLibrary() on the plugin file
 *   2. Calling ai_hands_plugin_abi_version() — reject on major mismatch
 *   3. Calling ai_hands_plugin_init() — receive plugin metadata + tool list
 *   4. Registering each tool name into the MCP tool dispatch table
 *   5. Dispatching tool calls via ai_hands_plugin_call()
 *   6. Calling ai_hands_plugin_shutdown() on unload
 *
 * ============================================================================
 * WHY HEAVYWEIGHT C-FFI (vs. lightweight Lua/Wasm)?
 * ============================================================================
 *
 * The C-FFI is the lowest-level interop boundary on every platform AI-Hands
 * targets. Any higher-level binding (Lua, Python, JavaScript, Wasm) can be
 * built as a wrapper that exposes this ABI from the host side, without
 * requiring changes to the core hands binary. Choosing C-FFI first keeps the
 * door open for every other plugin model. Choosing Lua first would have
 * locked plugins to that runtime.
 *
 * ============================================================================
 * THREAD SAFETY
 * ============================================================================
 *
 * `ai_hands_plugin_call` MAY be invoked from any thread, and MAY be invoked
 * concurrently from multiple threads. Plugins MUST be reentrant. If your
 * plugin holds internal mutable state, guard it with a mutex.
 *
 * `ai_hands_plugin_init` and `ai_hands_plugin_shutdown` are guaranteed to be
 * called exactly once per plugin lifetime, from the host's load/unload
 * threads. No tool calls will overlap with init/shutdown.
 *
 * `ai_hands_plugin_free_string` MAY be called from any thread, on any string
 * the plugin produced via `ai_hands_plugin_call`. The plugin's allocator
 * must support cross-thread free.
 *
 * ============================================================================
 * VERSION SKEW POLICY
 * ============================================================================
 *
 * AI_HANDS_PLUGIN_ABI_VERSION_MAJOR — breaking changes (struct layout,
 *   entry-point signature, status-enum reassignment). Host REJECTS plugins
 *   compiled against a different major version.
 *
 * AI_HANDS_PLUGIN_ABI_VERSION_MINOR — purely additive changes (new optional
 *   entry points, new status codes appended, new descriptor fields appended
 *   behind a length tag in future revisions). Host ACCEPTS minor skew but
 *   MAY emit a warning. Plugins SHOULD compile against the lowest minor
 *   version they require.
 *
 * ============================================================================
 * WRITING A PLUGIN
 * ============================================================================
 *
 *   1. Include this header in your plugin source.
 *   2. Export the 5 entry points listed at the bottom of this file.
 *   3. Build as a dynamic library (cdylib in Rust, /LD in MSVC, -shared in
 *      gcc/clang).
 *   4. Drop the resulting .dll/.so/.dylib in the host's plugin directory:
 *        Windows: %LOCALAPPDATA%\hands\plugins\
 *        Linux:   $XDG_DATA_HOME/hands/plugins/  (or ~/.local/share/hands/plugins/)
 *        macOS:   ~/Library/Application Support/hands/plugins/
 *   5. Restart hands.exe (or call hands_plugin_load with an absolute path).
 *
 * A minimal example plugin lives under
 *   installers/plugin-abi/example/
 * in the AI-Hands repository.
 *
 * ============================================================================
 */

#ifndef AI_HANDS_PLUGIN_H
#define AI_HANDS_PLUGIN_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------- Version constants ---------- */

#define AI_HANDS_PLUGIN_ABI_VERSION_MAJOR 1u
#define AI_HANDS_PLUGIN_ABI_VERSION_MINOR 0u

/* Pack major+minor into the uint32_t returned by
 * ai_hands_plugin_abi_version(): (major << 16) | (minor & 0xFFFF). */
#define AI_HANDS_PLUGIN_ABI_VERSION                                            \
    ((AI_HANDS_PLUGIN_ABI_VERSION_MAJOR << 16) |                               \
     (AI_HANDS_PLUGIN_ABI_VERSION_MINOR & 0xFFFFu))

/* ---------- Status codes ---------- */

/* Returned by ai_hands_plugin_call. Treat any non-zero value as a failure;
 * specific codes let the host produce better error messages. New codes will
 * only ever be APPENDED; existing values are stable across minor versions. */
typedef enum {
    AI_HANDS_OK                  = 0,
    AI_HANDS_ERR_UNKNOWN_TOOL    = 1,
    AI_HANDS_ERR_INVALID_ARGS    = 2,
    AI_HANDS_ERR_INTERNAL        = 3,
    AI_HANDS_ERR_TIMEOUT         = 4,
    AI_HANDS_ERR_NOT_IMPLEMENTED = 5
} AiHandsStatus;

/* ---------- Tool descriptor ---------- */

/* Describes one tool exposed by the plugin. The host copies these fields
 * into its own tool registry; the pointers must remain valid at least until
 * ai_hands_plugin_shutdown() returns.
 *
 * - name: tool name visible to the agent (e.g. "myplugin_do_thing"). Use a
 *   stable prefix tied to your plugin to avoid collisions.
 * - description: human-readable description shown in the MCP tool list.
 * - input_schema_json: JSON Schema (draft 2020-12 or 2019-09) describing
 *   the tool's input object. Must be a valid UTF-8 JSON string. */
typedef struct {
    const char* name;
    const char* description;
    const char* input_schema_json;
} AiHandsToolDescriptor;

/* ---------- Plugin info ---------- */

/* Returned by ai_hands_plugin_init. All pointer fields must remain valid
 * until ai_hands_plugin_shutdown() returns. The host does NOT free any of
 * these — the plugin owns them for its lifetime. */
typedef struct {
    const char* name;        /* e.g. "myplugin" */
    const char* version;     /* semver, e.g. "0.1.0" */
    const char* author;      /* free-form */
    const char* description; /* free-form */
    uint32_t tool_count;
    const AiHandsToolDescriptor* tools; /* array of tool_count items */
} AiHandsPluginInfo;

/* ---------- Entry points (exported by the plugin .dll/.so/.dylib) ---------- */

/* Returns the ABI version the plugin was compiled against. The host
 * compares the high 16 bits to AI_HANDS_PLUGIN_ABI_VERSION_MAJOR and
 * rejects the plugin on mismatch. */
uint32_t ai_hands_plugin_abi_version(void);

/* Called once after the plugin is loaded and the ABI version check passes.
 * Plugins should perform any one-time initialization here. The returned
 * pointer must remain valid until ai_hands_plugin_shutdown() returns.
 *
 * Returning NULL is treated as an init failure; the host will unload the
 * plugin. */
const AiHandsPluginInfo* ai_hands_plugin_init(void);

/* Invoke a tool by name.
 *
 * - tool_name:   UTF-8 NUL-terminated C string. Will match one of the
 *                names returned in AiHandsPluginInfo::tools.
 * - input_json:  UTF-8 NUL-terminated JSON object literal matching the
 *                tool's input_schema_json.
 * - output_json: out-parameter. On AI_HANDS_OK the plugin MUST set
 *                *output_json to a malloc'd UTF-8 NUL-terminated JSON
 *                string. The host will pass it back to
 *                ai_hands_plugin_free_string when done.
 *                On error the plugin MAY set *output_json to a JSON object
 *                describing the error, or leave it NULL.
 *
 * Returns one of the AiHandsStatus values.
 *
 * MAY be called concurrently from multiple threads. Plugins MUST be
 * reentrant. */
int32_t ai_hands_plugin_call(const char* tool_name,
                             const char* input_json,
                             char** output_json);

/* Free a string previously returned via the output_json out-parameter of
 * ai_hands_plugin_call. The host calls this on every non-NULL output_json
 * regardless of the AiHandsStatus value. */
void ai_hands_plugin_free_string(char* s);

/* Called once before the plugin is unloaded. The plugin should release any
 * resources it allocated during init or tool calls. After this returns,
 * the host will not call any other entry point on this plugin. */
void ai_hands_plugin_shutdown(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AI_HANDS_PLUGIN_H */
