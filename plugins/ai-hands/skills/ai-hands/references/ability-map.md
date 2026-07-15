# AI-Hands ability map

## Current-state abilities

- Browser lifecycle, navigation, extraction, accessibility, interaction, tabs, contexts, and visible verification
- Windows UIA discovery and interaction
- Screenshots, OCR, image comparison, template matching, and visual analysis
- Monitor topology and strict monitor scoping
- Current network logs, routes, traces, and API discovery
- Short in-process scripts coordinating current observation and action

Prefer `hands_*` meta-tools. Use browser, UIA, or vision primitives as evidence-driven escape hatches.

## Workflow-owned durable abilities

- API catalog, test, and direct call
- Credential and TOTP storage or refresh
- Recording and replay
- Adaptation history and reviewed adaptation
- Schedules, durable watches, and cross-visit procedure memory

Hands can discover evidence for these abilities, but Workflow owns persistence and reuse. Do not recreate removed `hands_self_record_*` front doors in host instructions.

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
