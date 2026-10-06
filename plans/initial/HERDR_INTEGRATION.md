# Herdr integration

Scope: supported plugin-v1 install/setup entrypoint, local topology discovery/recovery, durable native identity mapping and no-focus workspace creation.

[Main plan](GOATR_PLAN.md) · [Protocol/state](PROTOCOL_STATE.md) · [Pairing/security](PAIRING_SECURITY.md#revocation-fence) · [Companion configuration](COMPANION_PACKAGING.md#configuration-and-trust-roots) · [OMP](OMP_BACKEND.md) · [Codex](CODEX_BACKEND.md) · [Source anchors](REFERENCES.md#critical-files--anchors)

## Supported plugin-v1 entrypoint

Plan `plugins/goatr/herdr-plugin.toml` plus one executable Bash dispatcher `goatr.sh`; no new SDK, generic plugin layer, native GUI or Cargo package. Declare required `id = "goatr"`, name, a real release version, `min_herdr_version = "0.9.3"` and `platforms = ["linux"]`. Declare one action `install-setup` titled **Install / set up Goatr**, with argv `["bash", "goatr.sh", "open"]`, and one pane `installer`, `placement = "overlay"`, with argv `["bash", "goatr.sh", "pane"]`. Omit build/startup/events/link handlers entirely: registration and Herdr startup have no companion installation/start/pair side effects.

The `open` branch requires the injected `HERDR_BIN_PATH` and calls that exact running binary's `plugin pane open --plugin goatr --entrypoint installer --placement overlay --focus`; no guessed PATH binary, socket/session switch or daemon launch. The action does **only** this bridge and safe error reporting. Herdr captures action stdout/stderr, so prompts/setup/QR belong exclusively to the terminal `pane` branch; action invocation success means command launch, not successful install. Overlay is a supported interactive terminal and restores prior focus/zoom when closed; the flow must keep failures/results visible until explicit dismissal. [Packaging](COMPANION_PACKAGING.md#herdr-plugin-install-and-companion-lifecycle) owns every package/setup/update/removal effect.

Source baseline is Herdr `3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df`, whose Cargo version is0.9.3. Its minimum-version validation is **not** ABI/revision negotiation or proof every binary labelled0.9.3 includes this API. Before release/use, preflight the actual installed/running binary version/source provenance and real `plugin.list`, `plugin.action.list`/invoke and `plugin.pane.open` argv/environment/TTY behavior against this pin; absent/incompatible API fails before installation. Use pinned `docs/next` plus parser/schema/runtime/installer sources, not the stale versioned0.9.3 page's no-update/immediate-checkout-removal statements. Current source supports plugin update and leased checkout generations; new invocations use new files, running panes retain their old generation, and detached children are not protected beyond the parent/server lifetime. See [immutable evidence](REFERENCES.md#herdr-plugin-and-nix-retention-evidence).

The public origin `trevarj/goatr` is known. **Future-only distribution shapes**, requiring implemented plugin/package and an approved real immutable release commit in `$release_rev`:

```sh
herdr plugin install --ref "$release_rev" trevarj/goatr/plugins/goatr
herdr plugin link /absolute/reviewed/goatr/plugins/goatr
herdr plugin action invoke goatr.install-setup
```

The shorthand is supported `owner/repo/subdir`, not an arbitrary Git URL/imagined registry package. Remote install previews source/commands and requires consent; noninteractive `--yes` belongs only to an explicitly trusted isolated smoke run. Local link does not run builds and is the development path, not release provenance. Registration/enabled state is same-user global across named servers; both link/install can register offline, but the interactive pane requires the intended running Herdr session. Installing over a local link is refused: explicitly unlink first. Commit-pinned update cannot pick a newer release; explicitly reinstall `--ref` to a separately reviewed new commit. No pin is fabricated, no publish/push/topic change or installation is authorized by this planning edit.


## Public local API and topology recovery

Implement `companion/src/herdr.rs` using Tokio Unix streams against public local `herdr.sock`: one request/connection, string request ID, NDJSON result/error,1-MiB initial request cap and bounded reader; separate continuously drained `events.subscribe` connection. Discover default/named sessions using `herdr session list --json` plus configured exact host-visible socket paths, deduplicated by canonical socket/process identity. Validate same UID/private paths. No hidden render/client bridges, pane resizing, focus commands or raw network-exposed sockets.

Internal discovery record is `{serverId,displayName,socketPath,linuxBootId,pid,pidStartTicks,socketDevice,socketInode,incarnation,availability,reason?}`; paths/process evidence remain host-local. Obtain peer PID/UID using Unix peer credentials, verify `/proc/<pid>/stat` start ticks and `/proc/sys/kernel/random/boot_id`, and stat the connected canonical socket before/after snapshot. If namespace translation or process evidence cannot be established, mark unavailable/ambiguous, not a trusted guessed PID. Name/PID alone never proves identity.

Topology recovery is subscribe → wait `subscription_started` → fetch `session.snapshot` separately → replace. Events invalidate topology, not replayable mutations; serialize refreshes and repeat if invalidated while refreshing. `events_lost`, socket replacement/reconnect redo this sequence. No native snapshot/event global cursor exists. Coalesce invalidations, not identity checks.

## Durable identity and routing

Persist a random serverId for the configured/discovered logical server slot (canonical socket location and named-server label under this host); change its incarnation UUID when Linux boot/PID-start/socket evidence changes. serverId is a UI grouping, not evidence that old terminal IDs remain valid. Collision of two live sources claiming a slot is ambiguous and fails closed.

Persistent session key is `(hostId,serverId,serverIncarnation,terminal_id,providerOwnerIdentity,nativeConversationId)`. Allocate a random sessionId once for a proven key; retain across companion/provider-connection restart only if all native identity evidence still matches. Provider owner means OMP instance/process identity or Codex owner process/socket epoch, not merely native conversation ID. New owner, terminal, native thread/conversation or Herdr incarnation creates a new sessionId; preserve the old row offline while cached history/receipts require it. A reconnect to the same owner/conversation or OMP room generation change keeps sessionId but replaces generation UUID before writes. Companion boot replaces all active generations/handles. Pane/workspace/tab movement only changes routing and does not change identity.

Join using explicit Herdr `agent_session` path/ID plus process/relay proof described in the backend topics. Never match cwd/title/latest activity. Herdr-supported OMP/Codex panes without sufficient backend proof produce protocol `UnboundRow` with terminal/routing/title and setup/ambiguity reason; they are not fake sessions with mutable IDs. A proven but disconnected binding retains its sessionId as offline. Unbound rows outside configured projects are still visible: projects restrict creation only.

## Workspace creation

Before enabling creation, prove the installed launcher contract: pinned `agent.start` accepts `{name,kind,pane_id,args,timeout_ms}`, obtains `interactive_agent_executable(kind)` and submits shell-encoded argv to that pane. It has **no executable parameter**. Setup must identify the actual Herdr pane shell/startup environment and demonstrate that its `omp`/`codex` command resolves to the expected installed wrapper, including shell aliases/functions. Read `workspace.create` env handling and the chosen shell's startup rules, then run one owned no-focus creation smoke and capture wrapper/process evidence. Supplying `env.PATH` alone is not proof because shell startup may replace it. If exact resolution cannot be proved, disable creation for that backend/server with an actionable reason; do not inject shell commands, alter global shell configuration or bypass the wrapper. Existing proven sessions remain usable. This is a required runtime feasibility gate, not an assertion already verified by source inspection.

Create `workspace.create {cwd:<canonical configured project>,label:<name>,focus:false}`; retain returned workspace/root pane and resolve its terminal_id via authoritative snapshot. Validate project availability/wrapper access at dispatch. Name uses protocol label bounds; backend alias is `goatr-<provider>-<action UUID>` for correlation, never shell syntax. Server must already exist; creation never starts a different named Herdr server.

Use [CreationProgress and CreationResources](PROTOCOL_STATE.md#ordered-creation-progress), with intent persisted before each side effect and returned IDs before the next. Capture workspace/root pane/terminal, owner start if needed, native thread/session, relay binding and final ready Target. Native readiness includes the exact desktop terminal, not only a running helper. After a lost response, read-only reconciliation may identify the resource by recorded correlation; ambiguous effects stay uncertain. Partial workspace/thread/relay resources remain visible in action.status/UI, never implicitly retried, deleted or rolled back. Revocation fences each subsequent stage without killing earlier desktop work.

## Acceptance gates

See [Herdr plugin lifecycle](TESTING.md#herdr-plugin-installation-and-lifecycle), [live session and creation](TESTING.md#live-session-and-creation), [generation replacement](TESTING.md#reconnect-and-generation-replacement), [mutation receipts and restart](TESTING.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.
