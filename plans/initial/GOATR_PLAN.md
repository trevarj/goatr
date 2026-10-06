# Goatr: native mobile access to desktop Herdr sessions

## Context
Build a standalone Android app and Linux desktop companion in this repository, named **Goatr**. Users pair by scanning a desktop QR code; embedded iroh uses direct connections or public relay fallback, without user-managed DNS, domains, router forwarding, SSH, Termux or a VPN. The native mobile facade shares existing Herdr-hosted **stock OMP and Codex** sessions and supports session creation, conversations/tool activity, prompts, cancellation and the dialogs those stock protocols expose. The scope explicitly excludes Pi and patched agent runtimes, accepting desktop-only unsupported/custom dialogs; background delivery must offer persistent connectivity or ntfy.

This is the authoritative entry point for scope, implementation order and completion. Linked topic documents own their detailed requirements; [Testing](TESTING.md) owns verification commands and acceptance checks, and [References](REFERENCES.md) owns source-backed decisions and critical anchors. Read this plan first, then the assigned topic and its dependencies. New paths and wire names are proposed Goatr contracts, not existing upstream APIs.

## Boundaries and non-goals
Preserve reference source trees and all other workspaces. This implementation plan alone does not authorize commits, pushes, paid signup or production service installation; any such action requires separate authorization. Setup and service behavior described in the topics is intended product behavior, not authorization to install it on a deployment host during implementation.

Only these two backends appear in creation/settings. Existing Pi/other panes may be omitted from Goatr session inventory; do not advertise them as supported.

**Settled pairing authority:** each paired trusted phone may discover/read/control/receive notifications for **all supported same-user Herdr sessions on that host**. Configured projects restrict **new session creation only**, never existing-session inventory/history/control/notifications. Pair confirmation discloses this explicitly; no additional remote role/ACL product is being introduced.

Do not add a hosted Goatr account, patched agent runtimes, terminal-driven native control, alternate transports or unrelated product features. Stock protocol limitations must remain visible rather than be hidden by a different process/session or weaker security; see [source decisions](REFERENCES.md#source-backed-decisions) and the contingencies below.

## Topic index
| Topic | Authoritative detail |
|---|---|
| [Tech stack](TECH_STACK.md) | Repository layout, licenses, exact build pins, patched native bindings and endpoint/stream lifecycle |
| [Pairing and security](PAIRING_SECURITY.md) | Private identity storage, admin socket, QR capability, authenticated peers and revocation fence |
| [Protocol and state](PROTOCOL_STATE.md) | Exact v1 framing/envelopes/method table, generation guards, reducers, content handles and action receipts |
| [Herdr integration](HERDR_INTEGRATION.md) | Public topology API, recovery, identity mapping and no-focus workspace creation |
| [OMP backend](OMP_BACKEND.md) | Stock collaboration v3, registry/guest transport, native dialogs, local relay, launch and history limitations |
| [Codex backend](CODEX_BACKEND.md) | Shared owner/thread proof, native control/dialogs, independent relays, ordered creation and restart limits |
| [UI design](UI_DESIGN.md) | Compose navigation, native Markdown/tools/forms, drafts and Android state |
| [Background delivery](BACKGROUND_DELIVERY.md) | Foreground keeper or ntfy, durable probe handoff, authenticated hints and notification entry |
| [Companion packaging](COMPANION_PACKAGING.md) | CLI/Nix packaging, explicit setup, canonical configuration/trust roots and service ownership |
| [Testing](TESTING.md) | Proposed commands, isolated smoke resources and required behavior/evidence gates |
| [References](REFERENCES.md) | Original source-backed decisions, rejected alternatives and critical source anchors |

## Implementation phases and dependencies
Implement in the following order. Transport/build work may come first, but **the protocol/state contracts must be defined before adapter implementation or pairing/session/push control dispatch**. A working stream alone is not permission to invent wire behavior. Read [configuration and trust roots](COMPANION_PACKAGING.md#configuration-and-trust-roots) before discovery/launch work even though final packaging is phase 8.

| Phase | Work and dependency boundary | Acceptance gates in Testing |
|---|---|---|
| 1. Build and native transport | [Tech stack](TECH_STACK.md). Establish independent pinned inputs and matching secure bindings; no provider/control implementation yet. | [Native libraries](TESTING.md#native-libraries); [direct/relay transport](TESTING.md#direct-and-relay-transport), completed with real framed actions after phases 2–3 |
| 2. Contract and state | [Protocol/state](PROTOCOL_STATE.md), informed by [security](PAIRING_SECURITY.md), [Herdr identity](HERDR_INTEGRATION.md) and the pinned provider sources. Define schema, Kotlin/Python parity, reducer ordering, targets and receipts before consumers. | [Protocol fixtures](TESTING.md#protocol-fixture-parity); [mutation receipts](TESTING.md#mutation-receipts-and-companion-restart), completed against adapters in phase 5 |
| 3. Pairing and revocation | [Pairing/security](PAIRING_SECURITY.md), depending on phases 1–2. Implement local administration, scan/paste and authorization fencing; no session data before authorization. | [Pairing fixtures](TESTING.md#protocol-fixture-parity); [revocation fence](TESTING.md#revocation-fence); [camera/UI evidence](TESTING.md#emulator-and-camera-evidence), completed in phase 6 |
| 4. Desktop topology and identity | [Herdr integration](HERDR_INTEGRATION.md), depending on phases 2–3 and configured trust roots. Establish proven identities, recovery and receipt-backed creation steps without desktop takeover. | [Live-session identity/creation](TESTING.md#live-session-and-creation); [generation replacement](TESTING.md#reconnect-and-generation-replacement), completed with phase 5 |
| 5. Stock adapters | [OMP](OMP_BACKEND.md) and [Codex](CODEX_BACKEND.md), each depending on the phase 2 contracts and phases 3–4 authorization/identity. Keep provider-specific requests, uncertainty and service ownership; neither substitutes another process for live attachment. | [Live attachment/creation](TESTING.md#live-session-and-creation); [dialogs/cancellation](TESTING.md#provider-dialogs-and-cancellation); [reconnect/generations](TESTING.md#reconnect-and-generation-replacement); [receipts/restart](TESTING.md#mutation-receipts-and-companion-restart) |
| 6. Native facade | [UI design](UI_DESIGN.md), depending on pair/state contracts and actual adapter capabilities. Build navigation, content/forms, generation-scoped drafts and receipt-aware controls. | [Compose behavior](TESTING.md#compose-behavior); [owned-emulator/camera evidence](TESTING.md#emulator-and-camera-evidence); [dialog behavior](TESTING.md#provider-dialogs-and-cancellation) |
| 7. Background delivery | [Background delivery](BACKGROUND_DELIVERY.md), depending on phases 2–3 and 6. Implement explicit delivery choice, durable registration/probe verification and authoritative notification entry. | [Background flow/network changes](TESTING.md#background-delivery); [revocation fence](TESTING.md#revocation-fence); [notification UI entry](TESTING.md#emulator-and-camera-evidence) |
| 8. Usable companion and integrated evidence | [Companion packaging](COMPANION_PACKAGING.md), integrating all preceding phases. Finish explicit setup, packaging and independent lifecycles; execute the complete proposed suite with owned resources. | [Command suite](TESTING.md#proposed-command-suite); [isolated smoke ownership](TESTING.md#isolated-smoke-ownership); all [required behavior checks](TESTING.md#required-behavior-checks) |

The gates are cross-topic: a fixture-only phase cannot claim a later real-provider/UI gate passed. Complete each deferred integration gate once its dependencies exist; final completion requires all specified behavior and evidence, not just successful compilation.

### Bounded work packages and handoffs

Packages are implementation slices within the phases above, not a second project-management system. Each has one integration owner; another agent may implement a genuinely independent adapter only after the shared inputs are frozen. Deliver the named implementation files/fixtures and evidence, not a compiling scaffold. Topic contracts remain authoritative; do not duplicate enums/config/retention in package-specific designs.

| Package / owner | Required inputs and dependencies | Owned deliverables | Observable done / handoff evidence |
|---|---|---|---|
| 1A Native/build owner | Stack pins, FFI/core immutable sources, licenses | Locked Nix/Gradle inputs, minimal patch+Cargo.lock, one native artifact graph, matching Python/Kotlin bindings and context lifecycle | Both native outputs and full binding API build; advisory/manifest/JNA/ABI/alignment report; real selected direct/relay proof is completed after2–3, not claimed early |
| 2A Contract owner | Protocol, security, both pinned provider schemas, config | Closed v1 schema, Python/Kotlin types and canonicalization, finite forms/events/errors, shared boundary/hash fixtures | Same fixtures parse/hash/reject identically; exact method table and notification/read types available before any consumer dispatch |
| 2B State/storage owner | 2A and Herdr identity rules | Serialized reducers/snapshot handoff, target/content/cursor lifecycle, SQLite receipt/creation/attention/read state, finite privacy/cache policies | Fault-injection proves ordering/crash windows/no auto-replay; content/capacity/secret exclusion fixtures; adapter integration evidence deferred explicitly to5 |
| 3 Security owner | 1A transport and2A/2B | Local admin/CLI, key stores, QR/paste, bounded pairing/hello/connection admission, revocation fence | No pre-auth session reads; token response-loss recovery, duplicate connection and revoke-boundary fixtures; actual camera/UI evidence completed with6 |
| 4 Topology/config owner | 2A/2B,3, installed Herdr/wrappers | Minimal config parsing/setup preflight, discovery records, server/session mapping, no-focus creation stages, non-mutable unbound rows | Named-server/topology-loss/PID/socket/namespace fixtures; installed shell wrapper-resolution proof or precise creation-disabled blocker; no fabricated executable API |
| 5A OMP owner | 2–4 frozen types/identity/config, pinned collab source | Registry join/guest client, bounded loopback relay, history projection, native select/editor/prompt/abort, receipt-backed launch | Source-parity fixtures plus same desktop/native process proof, all-branches disclosure, no-focus create, reconnect/room-loss/dialog and crash evidence |
| 5B Codex owner | 2–4 frozen types/identity/config, pinned schemas/owner | Shared-owner adapter, finite native forms, short-path manifest/relay helper and independent service lifecycle, ordered creation | Real loaded/thread/terminal binding, no cold fallback, first-response/pagination/busy/restart evidence, no companion-owned TUI lifetime |
| 6 Android facade owner | 2A/2B,3 and real5 capabilities | Native Compose screens/forms/tools/Markdown, camera/paste, stale offline history, recovery drafts/receipts, local forget and notification entry | Executed instrumented Compose suite, real-APK sends/stops/forms and fresh PNGs, accessibility/IME/secret checks; no terminal facade |
| 7 Delivery owner | 2A/2B durable attention/read state,3 fencing,6 entry flow | Explicit persistent/ntfy modes, encrypted registration/probe state, approved-origin publisher and authenticated hint reconciliation | Runnable persistent/ntfy gates on owned device, probe/rotation/retry/revoke/process-death fixtures, actual delivery/click evidence with API limits |
| 8 Packaging/integration owner | All prior handoffs | Packaged CLI/optional independent units, rerunnable explicit setup, completed owned smoke/GUI runners and source notices | Entire Testing suite actually executed with exact commands/artifacts; external blockers remain named blocked, production resources untouched, no missing requirements silently waived |

Any implementation-time change to shared schema/config/identity/retention must update its owning topic, both consumers and the matching fixture in the same handoff. One missing runtime prerequisite does not justify a mock-only pass or reduced backend scope. The owner finishes all reachable work and reports the exact failed preflight/required evidence; the package is not done while its required gate remains blocked.

### Requirement-to-gate coverage

| Contract family | Required gate(s) |
|---|---|
| Every wire object/method/event/error, canonical hash, snapshot/sequence/unknown handling | [Protocol fixture parity](TESTING.md#protocol-fixture-parity) |
| Full-host authority, bounded admin/pairing, response-loss recovery, duplicate connections and revoke completion | Protocol fixtures + [revocation](TESTING.md#revocation-fence) |
| Native pins/build ownership/FFI compatibility/path observation/notices | [Native libraries](TESTING.md#native-libraries) + [direct/relay](TESTING.md#direct-and-relay-transport) |
| Config/runtime paths/wrappers/socket limits, proven identity, partial creation and live sharing | [Source parity](TESTING.md#provider-source-parity) + [live creation](TESTING.md#live-session-and-creation) |
| Native dialogs/forms/history/pagination, exact response/cancel semantics | Source parity + [dialogs](TESTING.md#provider-dialogs-and-cancellation) |
| SessionId/generation/boot transitions, source merge and handles | [Reconnect](TESTING.md#reconnect-and-generation-replacement) |
| Allowed durable transitions/pre-write windows/uncertainty/no replay/secret receipts | [Receipts/restart](TESTING.md#mutation-receipts-and-companion-restart) |
| All finite privacy/cache/retention/eviction bounds, recovery and offline forgetting | [Content/cache/privacy](TESTING.md#content-cache-and-privacy) |
| Capability UI/independent selection/Markdown/IME/accessibility/QR/real APK | [Compose](TESTING.md#compose-behavior) + [emulator/camera](TESTING.md#emulator-and-camera-evidence) |
| Durable attention IDs/read-watermarks, push/probe/MAC/rotation/retry/HTTPS, explicit mode | [Background](TESTING.md#background-delivery) + privacy + notification UI entry |


## Implementation-agent guidance
1. Read the assigned topic, its linked dependencies, applicable [source decisions](REFERENCES.md#source-backed-decisions) and [critical anchors](REFERENCES.md#critical-files--anchors) before coding. Verify the actual installed stock versions, wrapper/sandbox paths and upstream API shapes; distinguish proposed Goatr paths/contracts from existing APIs.
2. Preserve user work in this repository and every reference workspace. Reuse only the explicitly permitted seams/patterns; do not rewrite upstream/global settings, bypass wrappers, overwrite managed configuration or force desktop transitions. Commits, pushes, paid signup and production installation require separate authorization.
3. Implement against the authoritative topic contract rather than creating parallel definitions in adapters/UI. Keep protocol, identity, authorization, request ownership and receipt semantics consistent across all consumers. Coordinate changes to shared schema/state before implementing consumers.
4. Supply the checks/scripts specified in [Testing](TESTING.md), then execute the relevant gates and full integrated suite in the pinned environment when the implementation is ready. Use isolated owned resources and existing configured credentials without copying or logging secrets. Do not use a physical device without authorization.
5. Report blockers precisely: missing source/version compatibility, provider credentials, signing/host-broker prerequisites, user-service access or public relay availability. Finish reachable work, fail unavailable backend paths clearly and do not bypass wrappers/security, substitute mocks or claim unexercised behavior. A blocked required gate remains blocked, not passed.
6. Completion evidence identifies changed files, exact commands actually executed and their results, source/backend versions, provider PID/start/native conversation/Herdr identities, observed direct/relay paths, fresh emulator PNGs and camera coverage, plus any runtime limitations or remaining blockers. Never present proposed checks as already executed or alignment evidence as a real 16-KiB device run.

**Verification status: developer infrastructure only (2026-10-06).** The repository contains the pinned Nix flake/lock, core and optional Android/emulator tool shells, packageable Python namespace/non-native dependencies, shared Make/CI checks, license/provenance and developer/agent onboarding. The coordinator ran formatting, `make check` (Ruff, formatting and dependency-import smoke), and `nix build path:.#companion --no-link` successfully. Both Android/emulator shell derivations evaluated; the Android shell ran JDK21, Gradle9.8.0 and ADB35.0.2 successfully. See [Testing](TESTING.md#current-infrastructure-checks) for exact commands. No secure native migration, APK, provider, emulator or product runtime acceptance gate passed; the dependency smoke is not a product/native gate. CI is configured, not remotely exercised.

**Next handoff: package1A remains incomplete.** Its integration owner must retain the secure source/patch/Cargo.lock identity, rebuild the complete compatible Python/Kotlin API and both native outputs, establish the real Gradle modules/wrapper/Maven locks, and execute advisory/manifest/JNA/ABI/alignment/native-loading checks. Freeze2A contracts before consumers; real selected direct/relay framed actions complete only after2–3. [Testing](TESTING.md#current-infrastructure-checks) lists the infrastructure checks the coordinator can run now; the later product suite remains required, not implemented or passed.

## Assumptions & contingencies
- Stock OMP/Codex only and desktop-only unsupported/custom dialogs are explicit user choices. Do not reintroduce Pi, agent forks, terminal emulation or unrelated model/settings/file-browser features to work around unavailable APIs.
- Public iroh relays are the selected hobby/personal default, not a production SLA. On outage show unavailable and allow explicit retry; do not silently move to IRC, public HTTP tunnels, paid services or weaker authentication.
- Current installed wrappers/versions are checked during implementation. If a backend no longer matches the documented protocol, fail that backend clearly and retain working reachable work; do not change the globally installed version or fabricate compatibility. The deliverable is not complete until the selected baseline backend path is exercised.
- Legacy embedded Codex and unenabled OMP collaboration remain visible setup requirements. The user must deliberately finish current work and enable/relaunch where required; implementation never forces that transition.
- No consequential product question remains open after the user's full-host pairing decision. Engineering defaults (finite bounds, typed decimal values, keyed receipt fingerprints, one native fetch owner and explicit fail-closed preflights) are documented in their owning topics. If implementation reveals an actual conflict requiring a product/security tradeoff, ask that precise question instead of silently changing authority, provider scope or delivery mode.
- Required feasibility gates remain: Herdr shell resolution must demonstrably select installed wrappers; provider namespace evidence must prove exact terminal/owner joins; secure FFI upgrade must compile/load; Codex stock replay/order/busy behavior and real background delivery must be exercised. These are implementation preflights with stop conditions, not invitations to reopen settled choices or invented upstream guarantees.
