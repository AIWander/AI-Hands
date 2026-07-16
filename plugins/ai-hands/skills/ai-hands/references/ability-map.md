# AI-Hands ability map

## Current-state abilities

- Browser lifecycle, navigation, extraction, accessibility, interaction, tabs, contexts, and visible verification
- Windows UIA discovery and interaction
- Screenshots, OCR, image comparison, template matching, and visual analysis
- Monitor topology and strict monitor scoping
- Minimized current-network logs, routes, and endpoint-shape discovery; pattern redaction is defense in depth, not a secrecy guarantee
- Fixed `browser_batch` and `uia_batch` actions coordinating current observation and action

Prefer `hands_*` meta-tools. Use browser, UIA, or vision primitives as evidence-driven escape hatches.

## Workflow-owned durable abilities

- API catalog, test, and direct call
- Credential and TOTP storage or refresh
- Recording and replay
- Adaptation history and reviewed adaptation
- Schedules, durable watches, and cross-visit procedure memory

Hands can discover evidence for these abilities, but Workflow owns persistence and reuse. Do not recreate removed `hands_self_record_*` front doors in host instructions.

## Profile safety

- `default`: 104 operational tools plus the catalog, 105 entries; safe-advertised and recommended.
- `full`: 106 plus the catalog, 107 entries; safe-advertised.
- `strict`: 108 plus the catalog, 109 entries; safe-advertised.
- `compatibility`: 143 plus the catalog, 144 entries; unsafe debug escape hatch.

Compatibility-only raw/direct-fetch/value/trace/QR/event/native-plugin surfaces require three gates: `HANDS_TOOL_PROFILE=compatibility`, the matching process environment gate, and the matching per-call acknowledgement. The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call. Compatibility is not the normal speed path. Prefer visible navigation and structured text, `hands_find`, fixed batches, screenshots plus explicit verification, or bounded `uia_get_state`. Safe profiles reduce built-in first-party surface; they do not make Hands a general OS or secrecy sandbox.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

The Hands HTTP dashboard is off by default and requires `HANDS_ENABLE_DASHBOARD=1` to opt in.

## Accurate revisit rule

Stored procedure memory is optional evidence, not authority. If a site or app may have changed, inspect current state first. Reuse a Workflow procedure only when its preconditions can be checked and the result can be verified. Fall back to live Hands discovery when accuracy would otherwise drop.

## Qualified-name normalization

Treat these Hands spellings as the same logical namespace:

- `hands__<tool>`
- `AI-Hands__<tool>`
- `mcp__hands__<tool>`

One shared policy owner should normalize the name before applying rules. Host adapters must not implement competing policy logic.

## Isolation limit

The strict monitor fence validates physical identity, topology, coordinates, UIA bounds, and a unique visible browser-window title. A title match does not cryptographically prove a CDP target is the same HWND. Use a separate Windows session or virtual machine when the isolation boundary is security-critical.
