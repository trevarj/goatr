# Goatr engineering documentation

Goatr is an Android companion and Rust-native Linux service for discovering and
controlling the user's supported same-user Herdr sessions. It connects a paired
phone to actual stock OMP and Codex sessions through direct Iroh connections
with public-relay fallback.

Goatr supports existing-session discovery, history and tool activity, prompts,
Stop, supported native dialogs and no-focus creation in configured projects. It
does not provide hosted accounts, an alternate agent engine, terminal-driven
provider control, automatic retargeting, fabricated native state or a substitute
conversation runtime.

## Scope and boundaries

Only stock OMP and Codex are mutation backends. Other agents and ordinary shells
remain read-only native panes. A paired phone is authorized for each supported
same-user session on its host; configured projects limit new-session creation.

Session identity, generation, receipts, native ownership, retention and privacy
limits are contract requirements. A build, fixture, source review, screenshot or
isolated provider result is not live-provider or Android acceptance evidence.

Development APK feedback is currently allowed before full acceptance. Use
incremental builds and checks for changed behavior, disclose known failures, and
keep identity, permission, privacy and no-replay protections intact. Full clean
builds and exhaustive OMP + Codex + Android acceptance are deferred. See
[status](STATUS.md) for the available APK and limits.

## Documentation map

| Document | Purpose |
|---|---|
| [Status](STATUS.md) | Current artifacts, known failures and next work |
| [Acceptance](ACCEPTANCE.md) | Owned resources, runner commands and required evidence gates |
| [Compose interface rewrite](UI_POLISH_PLAN.md) | Android presentation scope, active goal and delivery order |
| [Contracts](contracts/) | Protocol, security, topology, providers, UI, packaging and delivery rules |
| [References](REFERENCES.md) | Source provenance, licenses and rejected alternatives |

## Contract map

| Contract | Owns |
|---|---|
| [Protocol and state](contracts/PROTOCOL_STATE.md) | Wire framing, reducers, generation fences, retention and receipts |
| [Pairing and security](contracts/PAIRING_SECURITY.md) | Pairing authority, authentication, revocation and local administration |
| [Herdr integration](contracts/HERDR_INTEGRATION.md) | Topology, durable identity, routing and no-focus creation |
| [OMP backend](contracts/OMP_BACKEND.md) | Stock collaboration, dialogs, relay, launch and history |
| [Codex backend](contracts/CODEX_BACKEND.md) | Shared owner/thread attachment, dialogs, relay and creation |
| [UI design](contracts/UI_DESIGN.md) | Android navigation, provider presentation, drafts and state |
| [Tech stack](contracts/TECH_STACK.md) | Pinned build inputs and native transport |
| [Companion packaging](contracts/COMPANION_PACKAGING.md) | Setup, configuration, retained outputs and lifecycle |
| [Background delivery](contracts/BACKGROUND_DELIVERY.md) | Persistent delivery and notification entry |

## Dependency order

1. Freeze protocol/state, then native transport, pairing and Herdr topology.
2. Implement OMP and Codex against those contracts.
3. Integrate Android against actual adapter capabilities.
4. Add background delivery and public plugin lifecycle as separate product work.
5. Complete packaged, owned-resource acceptance with the required evidence.

Read the assigned contract and its dependencies before changing code. Source
pins establish provenance; they do not establish compatibility or runtime proof.
