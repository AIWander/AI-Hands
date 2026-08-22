# Security Policy

## What AI-Hands is, in security terms

AI-Hands gives an AI host real control of a browser and a Windows desktop. That is
the product, and it is also the threat model. Treat it as a powerful local
capability with an audit trail, not as a sandbox.

**What does not happen.** The server contains no AIWander endpoint, no network
analytics, and no update or reporting call. Its instrumentation is a redacted
JSONL projection written to local disk, never transmitted. The HTTP dashboard is
off unless you set `HANDS_ENABLE_DASHBOARD=1`.

**What does happen, by design.** Driving a browser means contacting whatever site
you point it at. Navigation, downloads, fetches, and form submissions generate
real outbound traffic to third parties, and page content comes back into the
model's context. No configuration removes this: it is the function.

## What actually enforces

Three different things are often described as "security". They are not equal, and
only one of them is enforcement:

| Layer | Strength |
|---|---|
| Rust monitor fence | Real enforcement inside the server. Fail-closed and independent of any host. |
| Tool profiles | Real capability reduction. The default profile does not advertise the unsafe raw, direct-fetch, and native-plugin tools; reaching them needs the profile, a process environment gate, and a per-call acknowledgement. |
| Opt-in hook policy | Advisory defense in depth. A hook definition is not enforcement merely because it exists. It becomes a boundary only when the host trusts that exact definition, the runtime can block that event, and a harmless probe proves it fired. |

The hook policy denies plaintext secrets and raw network capture into durable
storage outright. For consent-class actions - financial, destructive, external
send, account, and permission changes - it asks for human confirmation rather
than blocking permanently, because no consent broker ships in this package and a
hook that is switched off protects nothing. Set `AI_HANDS_CONSENT_MODE=deny` to
restore hard blocking. Tool arguments and model-supplied booleans are never
consent in either mode.

## Out of scope

- **Operating-system isolation.** The account running the host defines the real
  access boundary. AI-Hands does not narrow it.
- **Page-level intent.** The policy inspects tool arguments, which the model
  chooses. It cannot tell a legitimate click from a costly one by coordinates.
- **Content you send outward.** Redaction covers the local audit projection, not
  what a page receives or what returns to the model.
- **Secret hygiene in the pages you visit.** Use isolated browser sessions and
  least privilege; browser and network observation can encounter credentials.

Use a separate Windows session or a virtual machine when the isolation boundary
is security-critical.

## Supported versions

The latest minor version receives security updates.

## Reporting security issues

Please open a [GitHub Issue](https://github.com/AIWander/AI-Hands/issues) or email
contact@aiwander.ai. For anything exploitable, prefer email over a public issue.
