# Goatr: native mobile access to desktop Herdr sessions

## Context
Build a standalone Android app and **Rust-native Linux desktop companion** in this repository, named **Goatr**. The companion is one Cargo package and calls iroh1.3.0 directly; Android remains Kotlin/Compose with the secure migrated FFI. Users pair by scanning a desktop QR code; embedded iroh uses direct connections or public relay fallback, without user-managed DNS, domains, router forwarding, SSH, Termux or a VPN. The native mobile facade shares existing Herdr-hosted **stock OMP and Codex** sessions and supports session creation, conversations/tool activity, prompts, cancellation and the dialogs those stock protocols expose. The scope explicitly excludes Pi and patched agent runtimes, accepting desktop-only unsupported/custom dialogs; background delivery must offer persistent connectivity or ntfy.

This is the authoritative entry point for scope, implementation order and completion. Linked topic documents own their detailed requirements; [Testing](TESTING.md) owns verification commands and acceptance checks, and [References](REFERENCES.md) owns source-backed decisions and critical anchors. Read this plan first, then the assigned topic and its dependencies. New paths and wire names are proposed Goatr contracts, not existing upstream APIs.

Provide an optional **supported Herdr plugin-v1** that makes Linux companion installation/setup accessible from one explicit interactive action. Its manifest plus one Bash dispatcher reuse the same Rust package, consented setup/serve/pair flows and independent services; no build/startup installation hooks. [Herdr integration](HERDR_INTEGRATION.md#supported-plugin-v1-entrypoint) owns the pinned API/manifest/distribution boundary; [Packaging](COMPANION_PACKAGING.md#herdr-plugin-install-and-companion-lifecycle) owns immutable retained Nix outputs, ownership, rerun/upgrade/removal. This is planned, not available software; README remains user-only and truthful about availability.

## Boundaries and non-goals
Preserve reference source trees and all other workspaces. This implementation plan alone does not authorize commits, pushes, paid signup or production service installation; any such action requires separate authorization. Setup and service behavior described in the topics is intended product behavior, not authorization to install it on a deployment host during implementation.

Only these two backends appear in creation/settings. Existing Pi/other panes may be omitted from Goatr session inventory; do not advertise them as supported.

**Settled pairing authority:** each paired trusted phone may discover/read/control/receive notifications for **all supported same-user Herdr sessions on that host**. Configured projects restrict **new session creation only**, never existing-session inventory/history/control/notifications. Pair confirmation discloses this explicitly; no additional remote role/ACL product is being introduced.

Do not add a hosted Goatr account, patched agent runtimes, terminal-driven native control, alternate transports or unrelated product features. Stock protocol limitations must remain visible rather than be hidden by a different process/session or weaker security; see [source decisions](REFERENCES.md#source-backed-decisions) and the contingencies below.

## Topic index
| Topic | Authoritative detail |
|---|---|
| [Tech stack](TECH_STACK.md) | Single Rust companion package, exact build provenance, independently locked secure transport graphs, Android bindings and endpoint/stream lifecycle |
| [Pairing and security](PAIRING_SECURITY.md) | Private identity storage, admin socket, QR capability, authenticated peers and revocation fence |
| [Protocol and state](PROTOCOL_STATE.md) | Exact v1 framing/envelopes/method table, generation guards, reducers, content handles and action receipts |
| [Herdr integration](HERDR_INTEGRATION.md) | Supported plugin-v1 API/manifest/distribution boundary, public topology API, recovery, identity mapping and no-focus workspace creation |
| [OMP backend](OMP_BACKEND.md) | Stock collaboration v3, registry/guest transport, native dialogs, local relay, launch and history limitations |
| [Codex backend](CODEX_BACKEND.md) | Shared owner/thread proof, native control/dialogs, independent relays, ordered creation and restart limits |
| [UI design](UI_DESIGN.md) | Compose navigation, native Markdown/tools/forms, drafts and Android state |
| [Background delivery](BACKGROUND_DELIVERY.md) | Foreground keeper or ntfy, durable probe handoff, authenticated hints and notification entry |
| [Companion packaging](COMPANION_PACKAGING.md) | CLI/Nix packaging, explicit plugin install/setup/upgrade/removal, retained owned outputs, canonical configuration/trust roots and independent service ownership |
| [Testing](TESTING.md) | Proposed commands, isolated smoke resources and required behavior/evidence gates |
| [References](REFERENCES.md) | Original source-backed decisions, rejected alternatives and critical source anchors |

## Implementation phases and dependencies
Implement in the following order. Transport/build work may come first, but **the protocol/state contracts must be defined before adapter implementation or pairing/session/push control dispatch**. A working stream alone is not permission to invent wire behavior. Read [configuration and trust roots](COMPANION_PACKAGING.md#configuration-and-trust-roots) before discovery/launch work even though final packaging is phase 8.

| Phase | Work and dependency boundary | Acceptance gates in Testing |
|---|---|---|
| 1. Build and native transport | [Tech stack](TECH_STACK.md). Migrate the foundation to Rust, establish direct secure iroh plus migrated Android bindings and compare shared transport/security provenance across independent locks; no provider/control implementation yet. | [Native libraries](TESTING.md#native-libraries); [direct/relay transport](TESTING.md#direct-and-relay-transport), completed with real framed actions after phases 2–3 |
| 2. Contract and state | [Protocol/state](PROTOCOL_STATE.md), informed by [security](PAIRING_SECURITY.md), [Herdr identity](HERDR_INTEGRATION.md) and the pinned provider sources. Define schema, Kotlin/Rust parity, reducer ordering, targets and receipts before consumers. | [Protocol fixtures](TESTING.md#protocol-fixture-parity); [mutation receipts](TESTING.md#mutation-receipts-and-companion-restart), completed against adapters in phase 5 |
| 3. Pairing and revocation | [Pairing/security](PAIRING_SECURITY.md), depending on phases 1–2. Implement local administration, scan/paste and authorization fencing; no session data before authorization. | [Pairing fixtures](TESTING.md#protocol-fixture-parity); [revocation fence](TESTING.md#revocation-fence); [camera/UI evidence](TESTING.md#emulator-and-camera-evidence), completed in phase 6 |
| 4. Desktop topology and identity | [Herdr integration](HERDR_INTEGRATION.md), depending on phases 2–3 and configured trust roots. Establish proven identities, recovery and receipt-backed creation steps without desktop takeover. | [Live-session identity/creation](TESTING.md#live-session-and-creation); [generation replacement](TESTING.md#reconnect-and-generation-replacement), completed with phase 5 |
| 5. Stock adapters | [OMP](OMP_BACKEND.md) and [Codex](CODEX_BACKEND.md), each depending on the phase 2 contracts and phases 3–4 authorization/identity. Keep provider-specific requests, uncertainty and service ownership; neither substitutes another process for live attachment. | [Live attachment/creation](TESTING.md#live-session-and-creation); [dialogs/cancellation](TESTING.md#provider-dialogs-and-cancellation); [reconnect/generations](TESTING.md#reconnect-and-generation-replacement); [receipts/restart](TESTING.md#mutation-receipts-and-companion-restart) |
| 6. Native facade | [UI design](UI_DESIGN.md), depending on pair/state contracts and actual adapter capabilities. Build navigation, content/forms, generation-scoped drafts and receipt-aware controls. | [Compose behavior](TESTING.md#compose-behavior); [owned-emulator/camera evidence](TESTING.md#emulator-and-camera-evidence); [dialog behavior](TESTING.md#provider-dialogs-and-cancellation) |
| 7. Background delivery | [Background delivery](BACKGROUND_DELIVERY.md), depending on phases 2–3 and 6. Implement explicit delivery choice, durable registration/probe verification and authoritative notification entry. | [Background flow/network changes](TESTING.md#background-delivery); [revocation fence](TESTING.md#revocation-fence); [notification UI entry](TESTING.md#emulator-and-camera-evidence) |
| 8. Usable companion and integrated evidence | [Companion packaging](COMPANION_PACKAGING.md) and [plugin-v1 entrypoint](HERDR_INTEGRATION.md#supported-plugin-v1-entrypoint), integrating all preceding phases. Finish the manifest/one Bash dispatcher, pinned distribution, explicit retained-output install/setup/upgrade/removal and independent lifecycles; execute the complete suite with owned resources. | [Plugin lifecycle](TESTING.md#herdr-plugin-installation-and-lifecycle); [command suite](TESTING.md#proposed-command-suite); [isolated ownership](TESTING.md#isolated-smoke-ownership); all [required behavior checks](TESTING.md#required-behavior-checks) |

The gates are cross-topic: a fixture-only phase cannot claim a later real-provider/UI gate passed. Complete each deferred integration gate once its dependencies exist; final completion requires all specified behavior and evidence, not just successful compilation.

### Bounded work packages and handoffs

Packages are implementation slices within the phases above, not a second project-management system. Each has one integration owner; another agent may implement a genuinely independent adapter only after the shared inputs are frozen. Deliver the named implementation files/fixtures and evidence, not a compiling scaffold. Topic contracts remain authoritative; do not duplicate enums/config/retention in package-specific designs.

| Package / owner | Required inputs and dependencies | Owned deliverables | Observable done / handoff evidence |
|---|---|---|---|
| 1A Native/build owner | Stack pins, FFI/core immutable sources, licenses | Rust Cargo package/companion lock and Nix/Make/CI cutover; separately migrated FFI patches/lock; generated Kotlin, intended Android artifacts and context lifecycle; exact shared transport/security manifest comparison | Rust executable and complete Kotlin-facing binding API build; both resolved graphs pass advisory/provenance checks; JNA/ABI/alignment/native-load report; full locks/host-Android ABI need not match; real selected direct/relay proof completes after2–3 |
| 2A Contract owner | Protocol, security, both pinned provider schemas, config | Closed v1 schema, Rust/Kotlin types and strict raw-token parsing/canonicalization, finite forms/events/errors, shared boundary/hash fixtures | Same fixtures parse/hash/reject identically; exact method table and notification/read types available before any consumer dispatch |
| 2B State/storage owner | 2A and Herdr identity rules | Serialized reducers/snapshot handoff, target/content/cursor lifecycle, SQLite receipt/creation/attention/read state, finite privacy/cache policies | Fault-injection proves ordering/crash windows/no auto-replay; content/capacity/secret exclusion fixtures; adapter integration evidence deferred explicitly to5 |
| 3 Security owner | 1A transport and2A/2B | Local admin/CLI, key stores, QR/paste, bounded pairing/hello/connection admission, revocation fence | No pre-auth session reads; token response-loss recovery, duplicate connection and revoke-boundary fixtures; actual camera/UI evidence completed with6 |
| 4 Topology/config owner | 2A/2B,3, installed Herdr/wrappers | Minimal config parsing/setup preflight, discovery records, server/session mapping, no-focus creation stages, non-mutable unbound rows | Named-server/topology-loss/PID/socket/namespace fixtures; installed shell wrapper-resolution proof or precise creation-disabled blocker; no fabricated executable API |
| 5A OMP owner | 2–4 frozen types/identity/config, pinned collab source | Registry join/guest client, bounded loopback relay, history projection, native select/editor/prompt/abort, receipt-backed launch | Source-parity fixtures plus same desktop/native process proof, all-branches disclosure, no-focus create, reconnect/room-loss/dialog and crash evidence |
| 5B Codex owner | 2–4 frozen types/identity/config, pinned schemas/owner | Shared-owner adapter, finite native forms, short-path manifest/relay helper and independent service lifecycle, ordered creation | Real loaded/thread/terminal binding, no cold fallback, first-response/pagination/busy/restart evidence, no companion-owned TUI lifetime |
| 6 Android facade owner | 2A/2B,3 and real5 capabilities | Native Compose screens/forms/tools/Markdown, camera/paste, stale offline history, recovery drafts/receipts, local forget and notification entry | Executed instrumented Compose suite, real-APK sends/stops/forms and fresh PNGs, accessibility/IME/secret checks; no terminal facade |
| 7 Delivery owner | 2A/2B durable attention/read state,3 fencing,6 entry flow | Explicit persistent/ntfy modes, encrypted registration/probe state, approved-origin publisher and authenticated hint reconciliation | Runnable persistent/ntfy gates on owned device, probe/rotation/retry/revoke/process-death fixtures, actual delivery/click evidence with API limits |
| 8 Packaging/integration owner | All prior handoffs; actual pinned-compatible Herdr/Nix preflights and reviewed release source | Packaged Rust CLI/optional independent units, `plugins/goatr` manifest/one Bash dispatcher, rerunnable explicit retained-output install/setup/staged upgrade/owned removal, completed existing-package smoke/GUI runners and source notices | Entire Testing suite including real local-link/public-pinned plugin lifecycle executed with exact registry/root/unit/process ownership, commands/source/artifacts; external blockers named blocked, production resources untouched, no silently waived requirements |

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
| Supported plugin-v1, pinned public/local distribution, explicit installation/serve/pair consent, retained GC roots, cancel/rerun/serialized upgrades, ownership/uninstall and independent lifetimes | [Herdr plugin installation and lifecycle](TESTING.md#herdr-plugin-installation-and-lifecycle) + live setup + receipts/restart + pairing/privacy |


## Implementation-agent guidance
1. Read the assigned topic, its linked dependencies, applicable [source decisions](REFERENCES.md#source-backed-decisions) and [critical anchors](REFERENCES.md#critical-files--anchors) before coding. Verify the actual installed stock versions, wrapper/sandbox paths and upstream API shapes; distinguish proposed Goatr paths/contracts from existing APIs.
2. Preserve user work in this repository and every reference workspace. Reuse only the explicitly permitted seams/patterns; do not rewrite upstream/global settings, bypass wrappers, overwrite managed configuration or force desktop transitions. Commits, pushes, paid signup and production installation require separate authorization.
3. Implement against the authoritative topic contract rather than creating parallel definitions in adapters/UI. Keep protocol, identity, authorization, request ownership and receipt semantics consistent across all consumers. Coordinate changes to shared schema/state before implementing consumers.
4. Supply the checks/scripts specified in [Testing](TESTING.md), then execute the relevant gates and full integrated suite in the pinned environment when the implementation is ready. Use isolated owned resources and existing configured credentials without copying or logging secrets. Do not use a physical device without authorization.
5. Report blockers precisely: missing source/version compatibility, provider credentials, signing/host-broker prerequisites, user-service access or public relay availability. Finish reachable work, fail unavailable backend paths clearly and do not bypass wrappers/security, substitute mocks or claim unexercised behavior. A blocked required gate remains blocked, not passed.
6. Completion evidence identifies changed files, exact commands actually executed and their results, source/backend versions, provider PID/start/native conversation/Herdr identities, observed direct/relay paths, fresh emulator PNGs and camera coverage, plus any runtime limitations or remaining blockers. Never present proposed checks as already executed or alignment evidence as a real 16-KiB device run.

## Current implementation status

Status as of **2026-10-06**. Checked items describe the generic developer
foundation and removal of obsolete Python tooling only. Earlier Python checks
are historical evidence, not verification of the current tool checks or Rust
product completion. Package completion still requires its named deliverables
and acceptance evidence above.

- [x] Generic developer foundation: pinned Nix flake/lock; core and optional
  Android/emulator tool shells; shared Make/CI entrypoints; license/provenance
  and developer/agent onboarding.
- [x] **Development-tool cutover:** removed the Python namespace, packaging and
  tools; Make/CI now share Nix formatting and genuine Rust tool-version smoke.
  `nix develop path:. -c make check` passed Nix formatting and executed
  Cargo/rustc1.99.0, rustfmt1.10.0-stable and Clippy0.1.99; no product gate passed.
- [ ] **Rust product foundation:** actual companion Cargo source/retained
  lock, direct iroh dependency graph, Rust Nix package and Make/CI product gates.
- [ ] **1A — Native/build:** secure Android FFI migration, complete generated
  Kotlin-facing API/artifacts, shared-provenance comparison and Gradle integration.
- [ ] **2A — Contract:** closed protocol/types and shared canonical fixtures.
- [ ] **2B — State/storage:** reducers, durable state and bounded privacy/cache.
- [ ] **3 — Security:** pairing, administration, authorization and revocation.
- [ ] **4 — Topology/config:** discovery, proven identities and no-focus creation.
- [ ] **5A — OMP:** stock live-session adapter and receipt-backed launch.
- [ ] **5B — Codex:** shared-owner adapter and independent relay lifecycle.
- [ ] **6 — Android facade:** real native app, controls and lifecycle behavior.
- [ ] **7 — Delivery:** persistent/ntfy modes and authoritative notification entry.
- [ ] **8 — Packaging/integration:** usable Rust companion/setup, supported plugin-v1 manifest/one Bash dispatcher, retained owned Nix outputs and consented install/upgrade/removal, integrated suite.

### Unexecuted product/runtime gates

These are required, **not passed**. [Testing](TESTING.md) owns the exact checks
and commands; tool availability and historical dependency imports do not satisfy them.

- [ ] Secure native builds/advisory/manifest/JNA/ABI/alignment checks and native
  loading; real 16-KiB runtime evidence remains unexercised.
- [ ] Protocol/state/security fixtures and authenticated selected direct/relay
  framed actions, including pairing and revocation.
- [ ] Real Herdr/OMP/Codex identity, creation, dialogs, cancellation, reconnect,
  generation replacement and mutation-receipt/restart gates.
- [ ] Content/cache/privacy and Compose behavior checks.
- [ ] Owned emulator, real APK, fresh UI PNGs and camera evidence.
- [ ] Actual persistent/ntfy delivery, network changes and notification entry.
- [ ] Full proposed command suite and isolated integrated smoke gates.
- [ ] Real plugin parse/action/pane and deployed-Herdr compatibility; local link/public immutable `--ref` distribution; consent/cancel/failure/concurrency/rerun/GC retention/upgrade/removal with isolated registry/roots/units and unchanged providers/state.

Plugin implementation/release preflights remain exact: `plugins/goatr` manifest/dispatcher, runnable Rust `goatr setup`/serve/pair and `packages.goatr`/`apps.goatr` are absent; an approved immutable public release commit has not been selected/published. The deployed Herdr binary's pinned plugin API/source match, supported Nix minimum/features/local-store root registration, isolated registry/wrapper-visible launch paths and owned user-unit environment still require runtime evidence. Upstream plugin-v1 supports the design; no unknown SDK/uninstall hook or fake release revision is a prerequisite.

Only Linux x86_64 availability was inspected in the foundation handoff, not
cross-platform execution. The emulator was not launched; `/dev/kvm` was not
visible on the inspected workstation. AGP/Kotlin/UI/JNA pins and upstream native
sources resolved during [availability preflight](TECH_STACK.md#foundation-availability-preflight),
but that is not proof of compatibility, API-37 compilation or secure native
migration. No Gradle modules/wrapper, Maven locks/verification files or native
outputs have been supplied.

**Historical verification status: retired Python developer infrastructure only (2026-10-06, before tooling removal).** At that time the repository contained the pinned Nix flake/lock, core and optional Android/emulator tool shells, packageable Python namespace/non-native dependencies, shared Make/CI checks, license/provenance and developer/agent onboarding. The coordinator ran formatting, `make check` (Ruff, formatting and dependency-import smoke), and `nix build path:.#companion --no-link` successfully. Both Android/emulator shell derivations evaluated; the Android shell ran JDK21, Gradle9.8.0 and ADB35.0.2 successfully. See [Testing](TESTING.md#historical-retired-python-foundation-verification) for exact retired commands. No Rust companion, secure native migration, APK, provider, emulator or product runtime acceptance gate passed; the dependency smoke was not a product/native gate. CI was configured, not remotely exercised. The subsequent tooling removal creates no Cargo sources/lock or product outputs and has not rerun or extended this evidence.

**Next handoff: Rust product foundation and package1A remain incomplete.** Implement the single Cargo package/retained companion lock, direct iroh1.3.0 Rust executable/Nix package and corresponding Make/CI product gates; the obsolete Python tooling has already been removed independently. Independently retain secure Android FFI source/patch/lock identity, rebuild its complete compatible Kotlin-facing API/intended Android outputs, compare exact shared transport/security provenance without requiring identical locks or host/Android ABI, establish Gradle modules/wrapper/Maven locks, and execute both-graph advisory plus manifest/JNA/ABI/alignment/native-loading checks. Follow [Tech stack](TECH_STACK.md) for the full contract; do not substitute the unexamined published1.1.0 AAR. Freeze2A contracts before consumers; real selected direct/relay framed actions complete only after2–3. [Testing](TESTING.md#current-infrastructure-checks) lists current development-tool checks; the later Rust/product suite remains required, not implemented or passed.

## Assumptions & contingencies
- Stock OMP/Codex only and desktop-only unsupported/custom dialogs are explicit user choices. Do not reintroduce Pi, agent forks, terminal emulation or unrelated model/settings/file-browser features to work around unavailable APIs.
- Public iroh relays are the selected hobby/personal default, not a production SLA. On outage show unavailable and allow explicit retry; do not silently move to IRC, public HTTP tunnels, paid services or weaker authentication.
- Current installed wrappers/versions are checked during implementation. If a backend no longer matches the documented protocol, fail that backend clearly and retain working reachable work; do not change the globally installed version or fabricate compatibility. The deliverable is not complete until the selected baseline backend path is exercised.
- Legacy embedded Codex and unenabled OMP collaboration remain visible setup requirements. The user must deliberately finish current work and enable/relaunch where required; implementation never forces that transition.
- No consequential product question remains open after the user's full-host pairing decision. Engineering defaults (finite bounds, typed decimal values, keyed receipt fingerprints, one native fetch owner and explicit fail-closed preflights) are documented in their owning topics. If implementation reveals an actual conflict requiring a product/security tradeoff, ask that precise question instead of silently changing authority, provider scope or delivery mode.
- Required feasibility gates remain: Herdr shell resolution must demonstrably select installed wrappers; provider namespace evidence must prove exact terminal/owner joins; secure FFI upgrade must compile/load; Codex stock replay/order/busy behavior and real background delivery must be exercised. These are implementation preflights with stop conditions, not invitations to reopen settled choices or invented upstream guarantees.
