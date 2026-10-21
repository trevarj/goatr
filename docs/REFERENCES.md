# References and source-backed decisions

Scope: original source decisions/rejected alternatives and critical upstream anchors. Re-read the relevant sources before implementing a topic; these references do not imply Goatr already implements their APIs.

[Documentation](README.md) · [Tech stack](contracts/TECH_STACK.md) · [Pairing/security](contracts/PAIRING_SECURITY.md) · [Herdr integration](contracts/HERDR_INTEGRATION.md) · [OMP backend](contracts/OMP_BACKEND.md) · [Codex backend](contracts/CODEX_BACKEND.md)

## Source-backed decisions

- Herdr `3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df` (0.9.3) supplies local NDJSON topology/control, not a universal conversation protocol. `agent.prompt` is PTY automation; do not use terminal reads/keys for the native conversation/approval path. Reference: https://github.com/herdrdev/herdr/tree/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df .
- The same pinned Herdr source supports plugin-v1 manifests/actions/interactive panes; no SDK or native nonterminal plugin GUI is needed. Goatr chooses one manifest/one Bash dispatcher, an explicit action-to-pane bridge and immutable retained Nix output builds, **not** install/startup hooks or a Nix profile installer. Actual deployed-binary/Nix compatibility and lifecycle are implementation gates; [plugin/retention evidence](#herdr-plugin-and-nix-retention-evidence) records source-versus-doc provenance.
- Stock OMP 18.6.1 collaboration protocol 3 already controls the exact running interactive process. Its persisted entries, transient events, prompt/abort and select/editor dialogs are reusable; custom dialogs, full rich-ask metadata and exact tool-approval arguments are not universally available. Read the installed `@oh-my-pi/pi-coding-agent` package's `src/collab/`, `src/modes/controllers/extension-ui-controller.ts`, and the resolved `@oh-my-pi/pi-wire` dependency's `src/index.ts`; locate the actual package roots in the deployment rather than assuming an npm prefix.
- **Original-host source refresh:** installed OMP and nested pi-wire now identify18.8.0, reviewed against maintainer pin `4ef97c8826ee012829a3e756b693a2a16a414f47` ([tag](https://api.github.com/repos/can1357/oh-my-pi/git/ref/tags/v18.8.0)). The public wire blob `d142c072b2ad70850fd9454e272cf686fbe0cbbb` is unchanged from18.6.1; relevant collaboration/persistence changes optimize serialization and writes, not the inspected guest grammar. Published archive SRI and installed publisher-owned source/dist comparisons are static identity evidence only, not registry-signature/SLSA validation or loaded-process/live compatibility. Keep18.6.1 as historical provenance; [OMP](contracts/OMP_BACKEND.md) records the reviewed current candidate and native source-elision correction.
- Stock Codex0.162.1, upstream `092d3acd6bec3e3a14bdc7e7a2810ab628ab759d` from annotated tag `7a078f6bb4ce0dc2a81408b334ce52979d92c603`, is the current signed Linux-x64-MUSL distribution baseline and supports concurrent clients of the same app-server. Its exact used Unix owner/listen/init, remote loaded/resume/paginated-turns, event/item and approval schema envelopes and root fields were compared with0.159.1; the latter remains historical provenance for the failed diagnostic. Existing embedded TUIs have no external attachment endpoint; never cold-resume them in a second process and call it live attachment. https://github.com/openai/codex/tree/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server .
- AgentWire is not needed as a deployed service or Python dependency. Its historical upstream Python reference at revision `68d1cb39ffddb5a39530ea8d918743f0452aedc0` informed provider-operation/receipt patterns and declares MIT metadata; the [public pinned source](https://github.com/trevarj/agentwire/tree/68d1cb39ffddb5a39530ea8d918743f0452aedc0) is reachable but not bundled here. Full applicable license/copyright notice remains unavailable: do not copy its code before recovering it. Independent implementation may follow the native provider contracts; do not import its IRC transport, channel-global binding, lossy approval projection or spawn-on-attach fallback.
- Roamgate `8de13620a3e9d22c7c40f95f01dd08b45d06b886` validates the shared-Herdr bridge model, but its mobile composer still sends pane input and its Ranger assistant is a separate session. Do not embed its terminal or substitute Ranger for the user's live agent. https://github.com/powerfooI/roamgate/tree/8de13620a3e9d22c7c40f95f01dd08b45d06b886 .
- SSH can carry structured data but does not remove reachability requirements; Termux RUN_COMMAND returns completion results rather than a duplex stream; Mosh synchronizes terminal screens and requires reachable UDP. Tailscale adds a phone VPN/app; Cloudflare Quick Tunnels are testing-only. These were investigated and are not additional Goatr transports.
- Iroh public relays are an accepted personal/hobby dependency with rate limits and no SLA. Traffic is encrypted end-to-end; relays/discovery still observe metadata. https://docs.iroh.computer/iroh-services/relays/public .

## Critical files & anchors

Evidence labels: **inspected source** means text/API/schema was read, not runtime proof; **implementation gate** means must be exercised later; **Goatr contract** means proposed behavior owned by another topic. Immutable source anchors below retain the original pins; local installed content is not assumed byte-identical to them.

1. **Iroh inspected source:** [FFI repository/revision](https://github.com/n0-computer/iroh-ffi/tree/5e451092dba0c1a09ee83ff6e5be37b1152a5c58), [endpoint path observation](https://github.com/n0-computer/iroh-ffi/blob/5e451092dba0c1a09ee83ff6e5be37b1152a5c58/src/endpoint.rs#L668-L682), [Android context JNI seam](https://github.com/n0-computer/iroh-ffi/blob/5e451092dba0c1a09ee83ff6e5be37b1152a5c58/kotlin/android/src/main/kotlin/computer/iroh/IrohAndroid.kt). Core v1.3.0 resolves through annotated tag `613ea3727b63de828e9eb65ad2f490d2b3181266` to commit `0072d7d84b233f9e7185eb676f049beaf557ac03`; [endpoint APIs](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs#L500-L528), [portmapper](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs#L790-L815), [network change](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs#L1658-L1682). Full Android FFI migration, independently resolved secure companion/FFI graphs, complete Kotlin-facing API/direct Rust build, and `endpoint_two_relay_only_no_ip` interoperability/runtime behavior remain implementation gates.
2. **Herdr inspected source:** [agent schema](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/api/schema/agents.rs), [actual launcher](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/app/agents.rs#L145-L231), [workspace schema](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/api/schema/workspaces.rs). Kind-based shell launch has no executable-path argument; environment/startup wrapper resolution must pass the owned creation gate.
3. **OMP inspected local source and immutable baseline:** v18.6.1 tag resolves to `2a2c6dcbbb558c0f8145f67f28b3370984f2bf60`. [Registry](https://github.com/can1357/oh-my-pi/blob/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60/packages/coding-agent/src/collab/registry.ts), [host](https://github.com/can1357/oh-my-pi/blob/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60/packages/coding-agent/src/collab/host.ts), [link/protocol](https://github.com/can1357/oh-my-pi/blob/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60/packages/coding-agent/src/collab/protocol.ts), [local relay](https://github.com/can1357/oh-my-pi/blob/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60/packages/collab-web/scripts/local-relay.ts), [title slot](https://github.com/can1357/oh-my-pi/blob/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60/packages/coding-agent/src/session/session-title-slot.ts), [persistence](https://github.com/can1357/oh-my-pi/blob/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60/packages/coding-agent/src/session/session-persistence.ts). Local registry59–112/169–207 confirms no pathname field; local title-slot85–119 and persistence12–15/95–108/183–184 confirm fixed first line and truncation. Installed-source parity, join evidence and protocol behavior are gates, not implied by a version string.
4. **Codex inspected schema:** [native ServerRequest discriminants](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/ServerRequest.json), [schema directory](https://github.com/openai/codex/tree/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json). Exact anchors: [command params](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/CommandExecutionRequestApprovalParams.json), [command result](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/CommandExecutionRequestApprovalResponse.json), [file params](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/FileChangeRequestApprovalParams.json), [file result](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/FileChangeRequestApprovalResponse.json), [permissions params](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/PermissionsRequestApprovalParams.json), [permissions result](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/PermissionsRequestApprovalResponse.json), [questions params](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/ToolRequestUserInputParams.json), [questions result](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/ToolRequestUserInputResponse.json), [MCP params](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/McpServerElicitationRequestParams.json), [MCP result](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/schema/json/McpServerElicitationRequestResponse.json). Schemas prove finite shapes only; real-owner replay/order/first-response/busy/loaded-resume behavior remains a gate.

   **Codex inspected experimental runtime:** stable generated command schemas omit fields enabled by Goatr's `experimentalApi:true` and are not the complete selected contract. Pinned [runtime params](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/src/protocol/v2/item.rs#L1593-L1610) define `additionalPermissions` and ordered `availableDecisions`; the [runtime producer](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server/src/bespoke_event_handling.rs#L671-L675) computes effective decisions and [sends both fields](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server/src/bespoke_event_handling.rs#L782-L806). Preserve a present list exactly; only absence uses [legacy defaults](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/protocol/src/approvals.rs#L351-L406), mapped through the [native decision enum/conversion](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/src/protocol/v2/item.rs#L63-L110). Complete requested permission scope must remain visible or the request is desktop-only. These source reads correct shape/default expectations, not runtime execution evidence.

5. **AgentWire inspected historical upstream Python reference (not bundled):** [public revision](https://github.com/trevarj/agentwire/tree/68d1cb39ffddb5a39530ea8d918743f0452aedc0), `src/agentwire/stack.py`; audit inspected42–95/655–788 and `backends/codex.py:189–217,330–412,1314–1385,1677–1702`. These `.py` filenames retain genuine source provenance, not planned Goatr modules. Public pinned metadata was retrieved during implementation preflight; full applicable license notice is still required before code reuse. Its unsolicited thread/started selection, TUI ownership, lossy approvals and unimplemented MCP are explicitly not Goatr contracts.
6. **MOTD reuse baseline (not bundled):** [public revision](https://github.com/trevarj/motd/tree/f348f44d3b5ca747e3a8751f13a277fd601ffd73), scanner `app/src/main/kotlin/io/github/trevarj/motd/ui/invite/QrInviteScannerScreen.kt` and the named QR seams under `invite/`. Public pinned license/scanner source was retrieved during implementation preflight. The observed local main differs from this baseline; do not silently copy current local changes. Gradle versions and flake split emulator shell informed stack pins. Preserve GPL copyright/license obligations when copying only the permitted seams; Goatr QR/camera runtime remains unexercised.
7. **Private deployment configuration inspected locally (unavailable public reference):** `modules/home/agent-sandbox.nix:563–593,705–740`, checkout HEAD observed `3b1c8f72a5c1c45bddd3861bbb778f631487e571`. The personal repository name, checkout root and SSH origin are omitted. The original origin anchor was not publicly retrievable during inspection (raw fetch404); no replacement public URL is asserted. Reproduction requires an authorized copy of that reference, while deployment preflight must inspect the actual installed wrapper. Local contents were read, not proven clean/equal to HEAD. These constraints are not universal Herdr behavior.

### Herdr plugin and Nix retention evidence

All Herdr links below use the inspected baseline `3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df`, declared0.9.3 in [Cargo.toml](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/Cargo.toml#L1-L4). These are source/document reads, not runtime passes:

- [Current pinned plugin docs](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/docs/next/website/src/content/docs/plugins.mdx), [manifest/parser/min-version check](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/app/api/plugins/manifest.rs#L12-L95) and [typed plugin/source/action/pane schema](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/api/schema/plugins.rs). Required metadata/minimum semantic version, Linux platform, argv commands, actions/panes and `source.resolved_commit` exist; minimum0.9.3 is not a source-revision/ABI guarantee. No uninstall hook exists in the manifest vocabulary.
- [Runtime](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/app/api/plugins/runtime.rs#L16-L157), [environment](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/app/api/plugins/env.rs), [pane implementation](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/app/api/plugins/panes.rs#L10-L44) and [action/unlink handlers](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/app/api/plugins/mod.rs#L113-L223). Ordinary same-user commands inherit environment/cwd and injected `HERDR_BIN_PATH`/socket/plugin roots; actions capture logs rather than provide interactive setup. Unlink leaves existing panes running. Plugin code is not sandboxed.
- [CLI install/update](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/cli/plugin.rs#L164-L449), [build execution](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/cli/plugin.rs#L1460-L1558) and [leased installations](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/plugin_installations.rs#L9-L70). GitHub shorthand `owner/repo/subdir`, `--ref`, preview/consent and current-source update exist; link/install register offline/global same-user, link omits builds, and current generations survive active commands/panes/servers, not arbitrary detached children. [Versioned0.9.3 docs](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/docs/versions/0.9.3/website/src/content/docs/plugins.mdx) are stale for no-update/immediate-checkout-removal claims; current pinned source/`docs/next` govern this plan and deployed behavior still must be exercised.
- [Plugin paths](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/plugin_paths.rs#L1-L74) and [config/state root resolution](https://github.com/herdrdev/herdr/blob/3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df/src/config/io.rs#L22-L91) support XDG-isolated registry/checkout/state tests. Debug builds use `herdr-dev`, release builds `herdr`; do not assume `HERDR_CONFIG_PATH` alone isolates plugins or directory creation enforces private modes.
- The official docs link a **community**, not Herdr-maintained, cookbook. Inspected pin `18709cdc851dd63ed0543eb8388343a5446fd8d8`: [manifest/action/pane](https://github.com/ogulcancelik/herdr-plugin-examples/blob/18709cdc851dd63ed0543eb8388343a5446fd8d8/github-link-preview/herdr-plugin.toml) and [Bash action-to-pane bridge](https://github.com/ogulcancelik/herdr-plugin-examples/blob/18709cdc851dd63ed0543eb8388343a5446fd8d8/github-link-preview/open.sh) demonstrate actual `HERDR_BIN_PATH plugin pane open --plugin ... --entrypoint ...`; do not copy its clicked-URL/PATH fallback into the Goatr installer.
- Public [trevarj/goatr](https://github.com/trevarj/goatr) exists; `plugins/goatr`, Rust package outputs and an approved release commit are proposed Goatr deliverables, not inspected released artifacts. Marketplace indexing/topic publication is neither needed for shorthand install nor authorized by this planning assignment.

Native Nix retention uses the inspected [Nix2.28 `nix build` manual](https://nix.dev/manual/nix/2.28/command-ref/new-cli/nix3-build), [GC roots](https://nix.dev/manual/nix/2.28/package-management/garbage-collector-roots.html) and [`--add-root` indirect-root lifecycle](https://nix.dev/manual/nix/2.28/command-ref/nix-store/realise.html#opt-add-root). `--out-link` is an output-link **prefix** with possible index/output suffixes; `--no-link` creates no retained link. The new CLI is experimental, so select/test the supported version/features/store contract during implementation rather than assert a minimum from docs.

Nix2.28.7 source pin `af8365d4b4bb75bfffbb8b5484ea4865e6dcb92b` independently confirms [local-filesystem build output-link creation](https://github.com/NixOS/nix/blob/af8365d4b4bb75bfffbb8b5484ea4865e6dcb92b/src/nix/build.cc#L119-L121) and [`createOutLinks` permanent-root registration](https://github.com/NixOS/nix/blob/af8365d4b4bb75bfffbb8b5484ea4865e6dcb92b/src/libcmd/command.cc#L373-L400). Indirect roots register the absolute link location: removing it releases that root, and renaming it loses the registration target. Keep per-release roots at final stable paths outside Herdr checkouts; atomically switch a separate owned activation pointer, retaining old/in-use roots. No profile installation, manual system-GC-root mutation or remote-only-store retention assumption. Actual GC survival/ownership/removal is a [required lifecycle gate](ACCEPTANCE.md#herdr-plugin-installation-and-lifecycle), not proven by these reads.


### Codex startup terminal signatures

The pinned [startup pump](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/tui/src/startup_draft.rs#L315)
emits the static message `terminal input stream closed during startup`. Its
[event stream](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/tui/src/tui/event_stream.rs#L218)
terminates on either reader error or EOF. The closed diagnostic family
`startup_terminal_event_stream_closed` therefore identifies a startup stream
failure, not physical stdin EOF or intentional cancellation. Adding that missing
classifier does not establish that any prior unclassified output contained it.


At Codex pin `092d3acd6bec3e3a14bdc7e7a2810ab628ab759d`,
[bootstrap contexts](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/tui/src/app_server_session.rs#L199)
and [typed request-error prefixes](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server-client/src/lib.rs#L127)
support closed visible-signature classifications for account transport/decode/
server errors, model/requirements bootstrap and thread lookup. The guarded
Terminal check matches only static contexts and adjacent subtype prefixes;
it retains no dynamic suffix. Conflicting families remain multiple. These
observations are not authenticated native error objects.

[Returned bootstrap/onboarding errors](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/tui/src/lib.rs#L1215)
are not guaranteed to reach the optional tracing sink. There is no universal
onboarding prefix, and the [remote request path](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server-client/src/remote.rs#L639)
has no account-request timeout wrapper. Generic timeout words, empty/missing
logs and unclassified visible output do not establish a bootstrap cause.

## Codux mobile-controller reference

Reviewed 2026-10-14: [Codux pinned source](https://github.com/duxweb/codux/tree/3f0856847b51700be8ce7fb8746d8c501aad10a8),
revision `3f0856847b51700be8ce7fb8746d8c501aad10a8`. This is a comparative
design reference, not a dependency, copied implementation or live acceptance
result. Upstream [2.0.3 release](https://github.com/duxweb/codux/releases/tag/v2.0.3)
claims remain separate from our source inspection and Goatr runtime evidence.

- [Mobile session switcher](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/widgets/components/terminal_switcher_screen.dart)
  and [runtime store](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/services/remote_runtime_store.dart)
  implement terminal/worktree groups and selected/loading/activity state.
- [Home runtime coordinator](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/screens/home/home_runtime_coordinator.dart)
  and [terminal binding coordinator](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/services/remote_terminal_binding_coordinator.dart)
  implement cached view, ownership transitions and baseline recovery.
- [Remote terminal pane](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/widgets/components/remote_terminal_pane.dart)
  implements IME insets and a bottom action surface;
  [voice overlay](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/widgets/components/voice_input_overlay.dart)
  implements permission/error, cancellation, transcript review and explicit send.
- [Protocol](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/docs/protocol.md)
  owns terminal output/sequence/baseline and viewport semantics. These cannot
  establish native OMP/Codex history, dialogs, receipts or notification behavior.

Goatr adapts the useful mobile presentation/continuity patterns through its
existing contracts. Herdr supplies desktop/session/terminal management; Goatr
does not acquire PTY, worktree, filesystem, credential or tunnel authority.
The [UI polish plan](UI_POLISH_PLAN.md#feature-matrix)
records approved refinements and clearly labels later investigations.

### Codux leads for native startup and recovery

Astra rechecked current main at the same pin on 2026-10-16. Its binding store
separates replacement from late metadata, and tests both [replacement](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/crates/codux-runtime-live/src/ai_runtime/store/tests/bindings.rs#L202-L277)
and [metadata backfill](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/crates/codux-runtime-live/src/ai_runtime/store/tests/bindings.rs#L322-L369).
Use that test distinction for Goatr’s missing-to-present native report versus
actual owner/thread replacement. Capture the original guard before choosing a fix;
a permissive metadata merge is not native authorization.

Its [identity resolution](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/crates/codux-runtime-live/src/ai_runtime/store/resolve.rs#L163-L178)
and [connection synchronization](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/lib/services/remote_connection_sync_controller.dart#L26-L103)
suggest focused delayed-event and per-connection readiness regressions. Preserve
Goatr’s Hello, inventory, original native scope and source-epoch barriers. Its
[connection-generation tests](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/apps/mobile/test/remote_connection_sync_controller_test.dart#L5-L55)
distinguish requests pending from snapshots received. Goatr already isolates each
connection’s pending requests and reducer; a delayed old-connection reply during
replacement is a useful missing regression, not an observed cause of the Android
timeout. Capture the false readiness condition before changing behavior.

Codux’s [OMP driver](https://github.com/duxweb/codux/blob/3f0856847b51700be8ce7fb8746d8c501aad10a8/crates/codux-runtime-live/src/ai_runtime/tool_drivers/omp.rs#L20-L35)
configures no lifecycle hook. Its file-backed probe cannot explain Herdr’s missing
startup report or distinguish deferred from rejected admission. Its rollout-file/
cwd fallback and file-history index do not establish Goatr native identity or
provider-cursor paging; no backend code or dependency is adopted.

## Android interface design

Goatr independently implements its Android interface with Jetpack Compose and
Material 3. Shared components are named by function. No external application
code, artwork or branding is imported.

The [feature matrix](UI_POLISH_PLAN.md#feature-matrix) owns console/host/session
management, chat and native forms, terminal controls, attachments, Skills,
artifacts, worktrees, Git review/references, notifications, routes and settings.
These are Goatr requirements; a visual component or upstream feature is not
implementation or runtime evidence. Herdr and provider contracts govern every
native operation. Stage/commit/push and arbitrary file management are outside
this interface feature scope.

## Provider Skills native boundary

Pinned native sources for Skills support:

- Codex092d3acd [skills/list producer](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server/src/request_processors/catalog_processor.rs#L515) takes cwds/forceReload and returns directory metadata/errors from current configuration. Use only the original accepted thread cwd and existing owner; this is not threadId-scoped loaded-skill state.
- The existing [running thread resume response](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server/src/request_processors/thread_lifecycle.rs#L732) returns top-level cwd from its accepted configuration snapshot. [Settings notifications](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server/src/bespoke_event_handling.rs#L1226) carry threadId and threadSettings.cwd; observe them before queued callbacks to invalidate stale catalogue authority. These are source-backed capture seams, not an implemented Skills result.
- Codex [mentions](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/skills/src/mentions.rs#L79) and [selection](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/skills/src/selection.rs#L33) support exact-path `[$name](native-path)` text; ambiguous bare names and unrepresentable paths must not become guessed insertions. Selection never submits.
- Codex [fs/readFile](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server/src/request_processors/fs_processor.rs#L64) is an unbounded host read with sandbox None, not the required permission-preserving bounded preview API.
- OMP4ef97c88 [collaboration host](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/collab/host.ts) uses promptCustomMessage; [interactive skill expansion](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/modes/skill-command.ts) is a different path. Collaboration-v3 has no catalogue/document channel. Extension getCommands exists locally, without an inspected authenticated remote seam.

Current owned-launch implementation pins the inspected local OMP18.8.7 public
CLI/package and source APIs in
[omp_skills_source.json](../companion/src/backends/omp_skills_source.json).
Selected source/object evidence SHA256
578253069858ad8f1c9247d234d2a5ca6f4e0a09f5d5b0cc77baaccf3bc114ba
is retained in .goatr-omp-skills-q6pmndll/omp-18.8.7-source-identity.json.
Public getCommands() enabled source:skill entries, sessionManager.getSessionId()
and session lifecycle support the new authenticated original-process extension
seam. No upstream implementation was copied; the extension is project GPL.
OMP18.8.0 history above does not prove this18.8.7 identity.

The catalogue and bounded complete OMP preview are source-reviewed/integrated;
no runtime compatibility pass is claimed. Native invocation and Codex document
authority remain open in the [UI plan](UI_POLISH_PLAN.md#native-skills-boundary).

## Rust library proposals

These are primary maintainer API/source references for the minimal choices in
[Tech stack](contracts/TECH_STACK.md#companion-rust-package-and-dependency-resolution),
**not installed dependencies or selected application version pins**. `latest`
documents capabilities only; implementation resolves compatible releases and
retains exact versions/features/source checksums in `companion/Cargo.lock`.

| Proposed library | Primary reference and bounded purpose |
|---|---|
| Direct iroh1.3.0 / Tokio | [Pinned core manifest](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/Cargo.toml), [Tokio API](https://docs.rs/tokio/latest/tokio/); endpoint APIs above and bounded async Unix/socket/channel/time work |
| serde / serde_json / toml | [Serde](https://serde.rs/), [serde_json API](https://docs.rs/serde_json/latest/serde_json/), [RawValue](https://docs.rs/serde_json/latest/serde_json/value/struct.RawValue.html), [toml API](https://docs.rs/toml/latest/toml/); typed values/configuration and raw-token preservation, not automatic duplicate-key validation or canonicalization |
| clap | [Maintainer API](https://docs.rs/clap/latest/clap/); finite CLI plus same-binary internal `codex-relay --binding PATH` |
| reqwest with Rustls | [ClientBuilder](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html); HTTPS-only publication, TLS verification, explicit no-proxy/no-redirect behavior, timeout and address-resolution controls |
| tokio-tungstenite with Rustls | [Client handshake over an existing stream](https://docs.rs/tokio-tungstenite/latest/tokio_tungstenite/fn.client_async.html); OMP WSS and Codex Tokio Unix-stream forwarding with explicit frame/queue bounds |
| axum | [Maintainer API](https://docs.rs/axum/latest/axum/); only required bounded local HTTP/WebSocket relay routes, with exact stock status/handshake behavior supplied explicitly |
| rusqlite | [Maintainer API](https://docs.rs/rusqlite/latest/rusqlite/); plain SQL/transactions off the reader with system SQLite from locked nixpkgs, not bundled or untracked SQLite |
| RustCrypto aes-gcm / sha2 / hmac | [AES-GCM including in-place operations](https://docs.rs/aes-gcm/latest/aes_gcm/), [SHA-2](https://docs.rs/sha2/latest/sha2/), [HMAC verification](https://docs.rs/hmac/latest/hmac/); stock OMP framing and distinct receipt/push keyed hashes, no handwritten crypto |
| qrcode | [Terminal Unicode renderer](https://docs.rs/qrcode/latest/qrcode/render/unicode/index.html); desktop QR with image features disabled, no PNG/image stack |
| base64 / uuid / url as required | [base64](https://docs.rs/base64/latest/base64/), [uuid](https://docs.rs/uuid/latest/uuid/), [url](https://docs.rs/url/latest/url/); strict canonical encodings/identities/origins, not extra configuration abstractions |

The inspected pinned core manifest uses edition2024/MSRV1.91 and already
requires Tokio, serde, reqwest0.13-family with `rustls-no-provider`/`stream`, url,
futures-util and tracing; axum is optional test support, not a stock production
relay implementation. These are compatibility candidates, not Goatr pins.
Its Rustls0.23.33 compatible requirement is not a secure resolved selection:
both actual locks must independently resolve the >=0.23.45 floor and satisfy
the advisory exclusions in Tech stack. The [separate pinned FFI manifest](https://github.com/n0-computer/iroh-ffi/blob/5e451092dba0c1a09ee83ff6e5be37b1152a5c58/Cargo.toml)
retains Android/JNI/UniFFI ownership; it does not require a shared application
workspace or host FFI runtime.

`RawValue` can preserve native numeric tokens but does not replace recursive
duplicate-key/limit validation. Standard JSON serialization does not implement
the Goatr canonicalizer; no float/fixed-precision shortcut may narrow MCP's
128-byte exact decimal contract. Reqwest's resolution controls must connect
only to per-attempt validated addresses while retaining URL hostname/SNI;
connection reuse/automatic retries must not bypass address validation,
revocation fences or the exact publisher retry schedule.

## Reuse and license obligations

Goatr's chosen license remains GPL-3.0-or-later. Before copying each permitted seam, record source revision/path and retain copyright/license/NOTICE obligations in distributions. OMP installed LICENSE was inspected as MIT with Mario Zechner/Can Bölük/Stencil Labs notices; preserve them when porting parser/crypto/relay mechanics. MOTD LICENSE text is GPLv3; inspect per-file headers/project grant before copying the three QR/lifecycle seams, preserve notices and supply corresponding modified source/build instructions. AgentWire's historical upstream Python `pyproject.toml:11` declares MIT, but no root LICENSE was located: recover/retain the pinned copyright and full license notice before porting its helper behavior into Rust. License metadata alone does not supply that notice.

FFI tree exposes MIT/Apache license files; select and retain the applicable upstream license/notice terms for patched binding sources. Verify Codex generated-schema license/notice provenance before checking in the small used fixtures/projections; do not import its whole schema as a runtime framework. Inspect transitive packaged-library notices for both Rust companion and Android graphs (including RustCrypto, SQLite, JNA/Markwon/UniFFI) during native/package gates. Reading Herdr/wrapper/reference behavior does not authorize copying unrelated implementation or deployment modules. No reference tree is modified.

## Source usage by topic

- Native build/transport: direct Rust Iroh core and secure Android FFI/JNA references in [Tech stack](contracts/TECH_STACK.md#patched-native-bindings); companion proposals above remain unpinned until its lock exists.
- Pairing scanner: MOTD QR seams and pinned revision in [Pairing/security](contracts/PAIRING_SECURITY.md#qr-scanning-and-paste-lifecycle).
- Topology/identity: Herdr public API and agent schema; [Herdr integration](contracts/HERDR_INTEGRATION.md).
- Plugin API/distribution: pinned Herdr manifest/schema/runtime/CLI/leased-generation and community bridge evidence above; [Herdr integration](contracts/HERDR_INTEGRATION.md#supported-plugin-v1-entrypoint). Companion immutable build/retained-root lifecycle and consent/removal belong to [Packaging](contracts/COMPANION_PACKAGING.md#herdr-plugin-install-and-companion-lifecycle), with real isolated evidence in [Testing](ACCEPTANCE.md#herdr-plugin-installation-and-lifecycle).
- OMP: installed collaboration/wire/extension UI sources, stock link parser and local relay; [OMP backend](contracts/OMP_BACKEND.md).
- Codex: pinned app-server and AgentWire method/relay references; [Codex backend](contracts/CODEX_BACKEND.md).

## Reference limitations

Installed-package locations and local-reference revisions record source provenance from the original plan; private deployment identifiers remain omitted. Those sources are not bundled Goatr inputs or proof that any available checkout matches the documented revision. AgentWire and MOTD public pinned sources are now reachable as recorded above; AgentWire's full notice and authorized access to the private deployment reference remain distinct prerequisites. Do not invent missing source or notice evidence. Check actual installed versions, wrapper paths, sandbox namespaces and source availability during implementation; apply the [scope and boundaries](README.md#scope-and-boundaries) if they differ. Core source line numbers are location hints for the pinned version, not stable APIs.

Source inspections record provenance and contract inputs, not a resolved application graph or runtime proof. Protocol, retention, manifests and work packages remain proposed Goatr contracts; feasibility and live-provider gates in [Acceptance](ACCEPTANCE.md) must not be relabelled as passed source verification.

## Material icon assets

The Android presentation uses official [Google Material Icons](https://github.com/google/material-design-icons/tree/49d4db35df873165d6bd6ba09b063c7dafbac2f4/src)
at revision `49d4db35df873165d6bd6ba09b063c7dafbac2f4`. SVG path data was
converted to local `ic_material_*.xml` Android VectorDrawables; no icon runtime
dependency was added. The shipped `material-icons-notice.txt` retains Google
attribution; `android-third-party-notices.txt` contains the Apache-2.0 license.

Pinned source families (each under `src/<family>/materialicons/24px.svg`):
`content/add`, `navigation/arrow_back`, `image/center_focus_strong`,
`communication/chat`, `navigation/close`, `content/content_copy`,
`hardware/desktop_windows`, `navigation/expand_more`, `navigation/menu`,
`navigation/refresh`, `content/send`, `action/settings`, `av/stop`,
`action/terminal`, `content/save`, `editor/mode_edit`, `action/delete`.
Exact upstream SVG hashes and URLs are retained in the implementation evidence
the retained `material-sources.tsv` asset-provenance record.
The four newer vectors retain raw SVGs/URLs/hashes in
`snippet-comfort-freeze-reviewfix-c863e5a1/SOURCE_PROVENANCE.txt`; independent
fetch/path matching is recorded in `official-icons-observed-6b36e0ae-8148-477d-a194-23b040c4b6ad/outcome.json`.
Source matching does not establish runtime rendering.

Additional notification/play vectors use the same pinned source revision:
[social/notifications](https://raw.githubusercontent.com/google/material-design-icons/49d4db35df873165d6bd6ba09b063c7dafbac2f4/src/social/notifications/materialicons/24px.svg)
and [av/play_arrow](https://raw.githubusercontent.com/google/material-design-icons/49d4db35df873165d6bd6ba09b063c7dafbac2f4/src/av/play_arrow/materialicons/24px.svg).
Raw source hashes c43e401f607f9c98627a7c3d6fa5c5a959498adfd419c6d6272c15c559b6965d
and 39d601c1d4199c5cadddf5f96a1f57157147d1eedc7a147cdadc29805c6d5fea
are retained in .goatr-notification-icons-source-HCmIPt beside the checkout.
The notification vector must use the exact pinned geometry; source matching
does not establish rendering or notification behavior.

The artifact browser adds pinned Material [folder](https://raw.githubusercontent.com/google/material-design-icons/49d4db35df873165d6bd6ba09b063c7dafbac2f4/src/file/folder/materialicons/24px.svg)
and [visibility](https://raw.githubusercontent.com/google/material-design-icons/49d4db35df873165d6bd6ba09b063c7dafbac2f4/src/action/visibility/materialicons/24px.svg)
VectorDrawables from that same revision. Raw SVG SHA256 values are
f287ca4017ccbe8ff3c562acdd036a3ae88acb8637fb713f0c65b1b2f9a35b61
and c95003a018f3776b3107d3a70057e18e7e1f1810bbcbadd32eebb04238c75bbf.
Original geometry, conversion evidence and existing Apache notices are retained
in .goatr-artifacts-xqki6q_0/material-sources and freeze-v1/HANDOFF.md. Rendering
and artifact behavior still require their own runtime checks.

## Android window color ownership

[Android Force Dark guidance](https://developer.android.com/develop/ui/views/theming/darktheme#force_dark)
documents automatic recoloring for light platform themes and the theme/view
opt-out. Goatr retains its platform parent under Theme.Goatr and sets
`android:forceDarkAllowed=false` in its API29+ variant because Compose owns the
palette. The Mi 9T comparison verified the theme-level change fixes the
observed pairing-label contrast; the prior decor-only and explicit-color
changes did not. This is no claim about other OEMs/configurations.

## Bounded repository reads

[Git status](https://git-scm.com/docs/git-status#_porcelain_format_version_2)
defines machine status, rename/unmerged records, branch and submodule fields;
its background-refresh guidance recommends disabling optional locks.
[Git diff](https://git-scm.com/docs/git-diff) documents numstat binary markers,
NUL path delimiters and external-helper/textconv controls. These consulted
formats guide the planned Changes parser; actual installed-Git compatibility
and guarded native behavior still require their own observed checks.

## Changes execution fence

[Linux ptrace](https://man7.org/linux/man-pages/man2/ptrace.2.html) documents exec
stops, syscall information, descendant/exit events and EXITKILL. [Kernel Yama](https://www.kernel.org/doc/html/latest/admin-guide/LSM/Yama.html)
documents policy that can prohibit child tracing. These guide the planned
Linux-only Changes runner; support must be exercised on the actual host. They
do not authorize capabilities, policy changes or a sandbox bypass.
