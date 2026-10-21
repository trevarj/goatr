# Stock OMP backend

Scope: the exact running OMP process through collaboration v3, local relay/launch integration, native dialogs and explicitly all-branches history.

[Documentation](../README.md) · [Protocol/state](PROTOCOL_STATE.md) · [Pairing/security](PAIRING_SECURITY.md#revocation-fence) · [Herdr integration](HERDR_INTEGRATION.md) · [Companion configuration](COMPANION_PACKAGING.md#configuration-and-trust-roots) · [Source anchors](../REFERENCES.md#critical-files--anchors)

## Baseline and existing-session discovery

The historical source baseline is OMP18.6.1 at [`2a2c6dcbbb558c0f8145f67f28b3370984f2bf60`](https://github.com/can1357/oh-my-pi/tree/2a2c6dcbbb558c0f8145f67f28b3370984f2bf60). Installed package metadata now declares18.8.0; its public source at [`4ef97c8826ee012829a3e756b693a2a16a414f47`](https://github.com/can1357/oh-my-pi/tree/4ef97c8826ee012829a3e756b693a2a16a414f47) and inspected installed sources are the current **source-reviewed candidate**, retaining the inspected collaboration v3 guest contract. This is not a live compatibility pass or proof that installed source, published tarball, dist and a running process are identical. Read the live package's `src/collab/{registry,protocol,crypto,relay-client,host,replication-shrink}.ts`, `src/session/session-persistence.ts` and resolved `@oh-my-pi/pi-wire/src/index.ts` before adapting `companion/src/backends/omp.rs`. Admission still requires the native registry/protocol/identity evidence below, not an exact package-version gate. Backend status reports only `collaboration 3`, not an unproved running-provider version.

Existing sessions require desktop `/collab start` (control, not view) or already enabled control auto-start. Implement authenticated registry IPC at `~/.omp/run/collab-hosts`: read only owner-private metadata/socket records, send `{v:1,token,op:"snapshot"}` then `{v:1,token,op:"link",access:"control",generation:G}` on separate connections. Respect the stock 4-KiB request/64-KiB response/1500-ms bounds. Compare returned instance/generation/session identity to the selected card. `omp collab list --json` and `omp collab link <instanceId> --json` are operator diagnostics, not a polling subprocess loop. Use instanceId rather than unqualified PID because sandbox PID namespaces differ. Registry discovery is not activation; relay-disconnected/read-only rooms cannot mutate. Never rewrite user settings or send `/collab` as a prompt; already-open unmirrored dialogs remain desktop-only.

Concrete live join: start with Herdr's exact OMP terminal/pane/workspace and `agent_session.kind:"path"` report, then enumerate authenticated registry snapshots and prove the registry socket's actual `SO_PEERCRED` process belongs to that terminal through Herdr-reported foreground/ancestry evidence and verified namespace PID mapping. Stock registry `pid` must equal that process's proved innermost `NSpid`; never index `/proc` using the advertised PID, scan globally for it, or accept a PID alone. Retain boot/PID-start/user/PID/mount namespace evidence, instanceId/generation/sessionId/cwd and exact alias/native report. Multiple proved terminal owners remain ambiguous, including processes loading the same native ID. Fetch the selected generation's authenticated control link, re-snapshot across link/welcome/FINAL boundaries, and require writable proto3 welcome.header.id/cwd to agree with registry sessionId/cwd plus the actual FINAL snapshot before Ready. Cwd/title alone never grants identity. Missing namespace/process/terminal proof stays Unbound/setup-required.

This registry-first authority applies to **all live OMP sessions**, not an entry-count or missing-file exception. Stock18.8.0 `SessionManager.create` allocates a pathname and real in-memory header but deliberately defers JSONL until an assistant message; registry snapshots use `getSessionId()`/`getCwd()` and the authenticated welcome uses `snapshotForReplication()`'s in-memory native header/entries. A zero-turn allocated pathname can therefore be absent in both host and provider views while that same native session is live. No stock startup flag forces interactive zero-turn persistence. Goatr must not warm up, write a header, open/resume a substitute, or launch another engine/TUI to satisfy its history reader. The file is an optional protected history witness, never the live identity grant; an existing unsafe/mismatched file still fails closed.

Setup rejection distinguishes an absent native report, a wrong-agent report and
a non-path report, in that order. Registry discovery still examines every candidate;
only after zero matches remain does it distinguish an empty registry, unavailable
snapshots, failed owner joins or both failure families. A singleton match remains
required and multiple matches remain ambiguous. These closed reasons disclose no
native values or raw errors and identify rejection families, not an initiating cause.

Producer anchors for that decision are the candidate's [lazy persistence gate](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/session/session-manager.ts#L1773-L1779), [in-memory replication snapshot](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/session/session-manager.ts#L3340-L3348) and [registry cwd/sessionId producer](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/collab/host.ts#L646-L664), also inspected in installed source. Source inspection is not loaded-process equality or native Ready evidence.

## Guest transport, prompt and native dialogs

Guest transport uses `tokio-tungstenite` WebSockets with Rustls for WSS plus RustCrypto `aes-gcm`. For an existing room use the origin from its authenticated registry-provided, parsed control link, **not** the configured new-room relay. The capability contains a32-byte room key and separate16-byte write token; keep both host-local and out of logs. Connect to `<parsed origin>/r/<roomId>?role=guest`. Binary records are u32 big-endian peer ID +12-byte unique IV + AES-256-GCM ciphertext with appended16-byte tag, empty AAD. Encrypt/decrypt bounded buffers in place where possible. Send encrypted `{t:"hello",proto:3,name:"Goatr",writeToken:<token>}` and require writable welcome. Install all snapshot chunks and the final-snapshot boundary before ready, queue subsequent native records in order. Persisted `entry` frames are ID/parentId-bearing history; transient `event` frames update activity/tools but never manufacture duplicate completed messages.

Use `{t:"prompt",text}` and `{t:"abort"}` directly. Busy prompt follows stock steer semantics; label this in the composer. Stop is `Stop current work`, not a fabricated turn-ID compare-and-swap. Dispatch errors and lack of positive prompt acknowledgment retain the [receipt semantics](PROTOCOL_STATE.md#mutation-receipts).

Render stock `ui-request` kind `select` as its title/options/descriptions/selection marker/checked indices, and `editor` as title/prefill with native editing. Reply `{t:"ui-response",reqId,value:<selected string or text>}`; cancel omits value. Validate selected option membership against the saved request. `ui-request-end` removes the card everywhere; late responses never target a newer request. Render sequential ask steps as sequential forms, not an inferred whole-questionnaire object. Stock Approve/Deny prompts are shown verbatim as desktop dialogs, not claimed to contain complete correlated tool arguments. Confirm/input/custom UI that stock collaboration does not mirror gets a clear desktop-attention notice when detectable, never guessed keyboard controls. Guest disconnect retains host requests; host room recreation may leave an existing dialog desktop-only and must invalidate its old mobile card.

## Collaboration link parsing

Port `parseCollabLink` from stock `src/collab/protocol.ts`, with fixtures for its actual normalization order: trim; replace `%23` with `#`; recognize bare `room#secret`/`room.secret` using `[A-Za-z0-9_-]{10,64}` and default `wss://my.omp.sh`; recursively parse browser URL fragments; normalize HTTPS/WSS to WSS and HTTP/WS to WS only for literal loopback; match `/r/<room>[.<secret>]`; use suffix or fragment as unpadded base64url and require decoded length 32 (view) or 48 (control). First 32 bytes are the room key, last 16 the write token. Never fetch the browser link or put its secret in the WebSocket URL/log. Preserve stock decoded-length behavior with parity fixtures rather than assuming canonical textual lengths.

## Local relay and new-session launch

New OMP sessions use the companion-local `axum` HTTP/WebSocket relay limited to the required routes, production default `127.0.0.1:7466`; existing rooms retain their published relay. Port routing from stock https://github.com/can1357/oh-my-pi/blob/v18.6.1/packages/collab-web/scripts/local-relay.ts . `GET /r/<room>?role=host|guest`, regex `^/r/([A-Za-z0-9_-]{10,64})$`; wrong path/role404, non-upgrade426, duplicate host close4009, guest without host close4004. Implement these status/handshake rules explicitly rather than trusting framework defaults. Allocate guest IDs from1, notify host peer-joined/peer-left, rewrite guest envelope peer IDs, route host peer0 to all guests or nonzero to one. Host disconnect sends room-closed and close4001; rejected duplicate host must not delete the live room. Reject Origin/unexpected Host; no access logs/compression. Native frame ceiling8 MiB, per-socket ordered queue max32 frames/16 MiB; close on overflow, require guest resnapshot, never silently drop. Max64 local rooms and32 guests/room; reject excess. These adapter-native limits differ from the smaller Goatr phone-frame/queue limits.

Derive integration paths, provider-visible paths and relay URL from [runtime configuration](COMPANION_PACKAGING.md#configuration-and-trust-roots). Production overlay is private0600 `<runtime_dir>/omp-collab.yml` containing `collab:\n  autoStart: control\n  relayUrl: ws://127.0.0.1:7466\n`; tests substitute their owned bound port in **both** relay and overlay. Default port occupied fails visibly, never silently chooses another relay. OMP supports a per-process `--config <YAML path>` overlay after global/project settings; never modify global settings.

After the relay and [Herdr wrapper-resolution preflight](HERDR_INTEGRATION.md#workspace-creation) are ready, call `agent.start {name:<correlation alias>,kind:"omp",pane_id:<created pane>,args:["--cwd",<project>,"--config",<provider-visible overlay>],timeout_ms:30000}`. Require matching Herdr native-session report, proven registry instance and final writable snapshot. Capture all returned identities in creation progress. Deployment prerequisites are provider access to the companion's loopback relay and the configured project/integration paths; verify the installed wrapper's network namespace and mounts rather than assuming the originally inspected layout. Real launch proof remains required. Companion restart may recreate relay rooms without killing OMP; that replaces generation and may leave an old unmirrored dialog desktop-only.

## Persisted history and branch disclosure

Read persisted history only through the exact Herdr-reported `agent_session.kind:"path"` file, whose complete native session-header ID must equal the already-proved registry sessionId and authenticated welcome header.id. Before the leaf exists, prove its canonical protected allowed-root parent, stable same-user process and equal host/provider parent objects; accept absence only when both namespace views prove ENOENT, never a dangling symlink, permission failure or unknown view. Publish authoritative collaboration history as `live_only` with no fabricated header/history/native older-page cursor. On actual materialization, securely capture the real read-only file, namespace-object join and matching complete header before projecting/paging it; switch coverage to `persisted_and_live` without changing NativeId, Target/generation/source epoch or resending native input. Validate owner/type/canonical allowed session root/nonsymlink; no phone-supplied file paths. Parse the optional256-byte title slot then session header; retain completed JSONL records and tolerate an incomplete tail. Captured-file disappearance, replacement, shrink or changed header invalidates the source, not a return to lazy absence. Use native id/parentId to merge full persisted content with collab entries; never invoke writable SessionManager.open merely to inspect it. Known profile blob hashes may be resolved only under its known blobs directory; unknown roots/content are unavailable, not guessed.

Merge using protocol HistoryRecord/Content, preserving full persisted fields over collab placeholders and newer live tool/status updates over older persisted status. Parse read-only streams incrementally, never truncate/repair a provider file; partial last line waits for completion. Inode/size regression or changed header resets source epoch/cursors/content handles and rebuilds safely. Unknown entry types become unsupported records with safe original data; profile roots are local configured trust roots, not the project-creation allowlist. Keep source persistence truncation visible (inspected source truncates ordinary oversized strings), even when a Goatr content handle can carry more.

Always label OMP history **“All branches; desktop-selected branch unavailable.”** Stock tree navigation can move the live leaf without persisting an entry; neither last JSONL row nor last collab entry proves the current branch. Do not infer it. Keep native source truncation/placeholder markers visible; even local persistence can truncate large strings. The phone controls the selected exact live OMP session, not an inferred branch.

The source truncation contract comes from the candidate's pinned [replication shrinker](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/collab/replication-shrink.ts#L81-L115), [string/array/depth/cycle emission](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/collab/replication-shrink.ts#L169-L219) and [whole-entry placeholder](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/collab/replication-shrink.ts#L372-L385):1-MiB replicated-payload ceiling, depth1000, progressive string caps64KiB→64 chars (80-char reserve) and array limits256→1. Exact visible `…[N chars elided for collab session]`, `…[N items elided for collab session]`, `…[deeper levels elided for collab session]` and `…[cyclic reference elided for collab session]` mark projected text/thinking/tool-input content `sourceTruncated:true` without dropping the surviving content. Standalone marker elements beside surviving content blocks remain visible. Typed `collab-entry-too-large` placeholders retain native entry identity and are also flagged. These producer markers predate18.8; the correction addresses a pre-existing Goatr projection bug, not a changed wire/schema contract. A literal copy of an exact marker cannot be distinguished from producer elision by this stock representation; ordinary collaboration prose is not a marker.

The pinned [persistence bounds](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/session/session-persistence.ts#L12-L38) and [truncation/signed exceptions](https://github.com/can1357/oh-my-pi/blob/4ef97c8826ee012829a3e756b693a2a16a414f47/packages/coding-agent/src/session/session-persistence.ts#L168-L228) retain the500,000-character ordinary-string limit and visible `[Session persistence truncated large content]` notice. Detection examines only displayed text and redacted tool-input values; credentials/signatures/encrypted fields stay excluded. Goatr's native byte/depth/member limits and raw numeric-token preservation are unchanged; upstream JSON number rounding cannot be recovered. `companion/tests/omp.rs::stock_collab_elision_markers_reach_history_without_losing_content` exercises authenticated consumer history, genuine marker excerpts, normal controls and the typed placeholder. Focused failing-before/passing-after, relevant OMP tests/Clippy and actual stock-provider smoke are coordinator-owned checks, not runtime claims from source review.

Source-parity fixtures are mandatory: registry owner/privacy/4-KiB/64-KiB/1500-ms limits and identity races; duplicate native IDs across processes; link normalization; AES framing and malformed envelopes; hello/welcome/chunk/final ordering; select/editor/cancel and room-recreation loss; local-relay routing/duplicate host/overflow; optional256-byte title slot, partial JSONL tail, replacement and source truncation. [Testing](../ACCEPTANCE.md#provider-source-parity) names the runnable owners; fixtures prove Rust projection parity, real-provider smoke proves live attachment.

The installed periodic registry snapshot has one narrow transient outcome: its
existing1500-ms deadline enters observation-pending for the same authenticated
source. It retains the guest/read path, Target, source epoch, history and dialogs
while atomically making the session Synchronizing and denying mutation admission.
Only a complete later same-binding registry/owner/control/relay observation clears
the gate. Initial/link snapshots, malformed or replacement registry data,
owner/room/control loss and reader failure still invalidate the source. One
observation is in flight and missed ticks are skipped; no write is retried.

The concrete consumer regression `backends::omp::tests::zero_turn_native_authority_materializes_history_without_rebinding` uses the existing private-socket/kernel/authenticated-guest fixture to follow an absent-file zero-turn session through empty FINAL, live history, secure materialization, native file pagination and later append with the same Target/NativeId/source epoch, then rejects unsafe/replaced history. `companion/tests/omp.rs::allocated_history_path_rejects_unsafe_or_wrong_native_materialization` covers absent/no-synthetic-header behavior, dangling symlink/public file/wrong native header and captured disappearance. Registry re-snapshot coverage rejects cwd changes while allowing transient busy changes. These runnable checks are synthetic consumer evidence only; the stock-native runtime gate remains mandatory and requires coordinator validation.

## Concrete source implementation and frozen inputs

`companion/src/backends/omp.rs::Runtime` is owned by ordinary
`transport::serve` whenever a real `Topology` is present; transport-only smoke
still passes None. Discovery is registry-first using private registry files and
actual connected Unix peer/process evidence, reporter namespace/foreground
ancestry, exact terminal/alias/native report, control-link re-snapshot and
authenticated guest header.id/cwd plus FINAL. An optional file can witness
history only after its protected path/header and host/provider object join.
Duplicate proved terminal owners stay ambiguous. No activation subprocess,
project inventory filter, cold substitute, fabricated header or guessed
PID/cwd/title join exists.

`Link::{parse,seal,open}` preserve stock normalization/decoded-length behavior.
The retained AES-GCM cipher encrypts/decrypts in place; normal Rustls WebPKI
roots verify WSS. Frames cap8MiB; native JSON separately caps8MiB/depth128/
131072 members and preserves signed/integer/exponent tokens and literal marker
maps. Phone Uint grammar is unchanged. A writable proto3 welcome and the
actual FINAL chunk are mandatory even when entryCount is0 or already reached.
Queued native state/history/forms are installed under the captured actual
Target/epoch before readiness. Corrupt authentication is fatal for that room
generation; replacement restores reads only and never retargets queued writes.

The concrete `Relay::bind/run` uses production127.0.0.1:7466, exact Host,
no Origin, explicit404/426, no compression/logging,64 rooms/32 guests,
8MiB frames and per-socket32-frame/16MiB ordered budgets. Overflow closes for
resnapshot; rejected duplicate hosts do not remove the real owner. `overlay`
creates/verifies only the fixed private0600 YAML, with the actual test-bound
port when testing and no occupied-production-port fallback.
Guest forwarding and cleanup require the same socket UUID as the registered
numeric guest ID; delayed old-room sockets cannot impersonate/remove a new peer.

`HistoryFile` retains the exact allocated trusted read-only pathname, proved
parent/process and expected native ID even before native materialization, with
no synthetic header/records. Secure appearance captures the actual file/header/
title slot, complete-line offsets and projected record counts under the same
binding/source; partial tails wait. Any unsafe/wrong-ID present file fails
closed; captured disappearance/replacement/truncation/header changes invalidate
source/cursors/content rather than regain absence permission.
Native older reads use existing opaque source-scoped State page tickets and
an indexed entry/sub-record continuation, including overlap if retention cuts
through a single entry with many tools. Persisted full content wins replicated
placeholders; newer live tool/status wins older files. Unknown entries retain
safe unsupported originals. Credentials/signatures/encrypted thinking/pending
originals never enter display persistence; only a configured verified profile
blob root can resolve a canonical lowercase SHA256 blob. No remote image fetch.
The continuation can cross the live1000-row window while the phone retains only
its current bounded local display window; only the guarded native terminal cursor,
expiry or explicit resource limit ends that browse view.
Expired/evicted completed Content refs are reprojected from that exact guarded
entry/tool sub-record on both history refetch and session.open. Only expired
unchanged fields are replaced; native identity/parent links and live-newer tool
state/output remain intact, without generation/source-epoch rotation or another
cache. Offline/dirty topology does not authorize native rehydration.
Retained replacements enforce the shared64-MiB history budget before publishing,
counting distinct current and cursor-pinned vectors; excess pins explicitly expire.

This bounded cutover changes `companion/src/backends/omp.rs`, `companion/tests/omp.rs`, the OMP-only proof branch in `companion/examples/smoke/live.rs` and this topic contract. `HistoryFile::open` remains strict existing-file inspection; `HistoryFile::live(path, roots, owner, session)` additionally retains proved lazy absence, and the local `RegistrySnapshot.cwd` field participates in `same_binding`. Existing `Link::hello` is now public with unchanged signature/behavior so the maintained helper can authenticate without a second secret decoder or fabricated header. Wire/config/state/creation APIs, native write/revocation/once-only fences and provider ownership are unchanged. Synchronous adapter locks use the already-retained parking_lot Mutex; the WebSocket writer remains a Tokio async Mutex.

The maintained helper's OMP `NativeProof` starts only from an already-Ready authenticated phone binding, then independently joins the private registry peer/process/terminal and actual writable native welcome.header.id/cwd plus `SnapshotBoundary` FINAL. Registry/owner/topology are rechecked across that boundary. It retains that one independently authenticated guest and continuously drains bounded frames without retaining private event/dialog payloads. One command expectation is registered before each owned history input; exact unique live bashExecution command/marker/ID/success is independently joined to the same completed ID in Goatr Ready/Idle. Stream loss, malformed/authentication failure, duplicate welcome/FINAL/completion, source change or deadline permanently fails the original observer, never transparently rejoins. Fresh registry/process/topology/object checks still surround every native input. The captured `HistoryFile::live` witness rechecks its protected metadata/header and incrementally scans appended bytes, including one legitimate lazy materialization; it never reconstructs all growing history at each fence. No raw native entries, room secrets or control links enter evidence. Explicit existing companion recovery creates a new observer only after proving the unchanged native engine/owner/conversation for the newly observed Target. Runtime evidence remains separately recorded in Status; implementation and regression checks do not pass native acceptance.

Native select projection requires radio|checkbox selectionMarker and preserves
ordered duplicate options/checked markers, optional helpText and markableCount.
Native omitted marker projects radio. Unrepresentable/secret/oversized cards
remain wholly desktop-only. Editor prefill, single offered-string submit and
cancel-with-value-omitted preserve exact numeric native request IDs in memory.
Dialog closure never acknowledges a winner. Prompt/steer/current-work abort
and answers use tracked once-only original jobs and actual underlying WS TCP
FD fences; write completion ends Dispatched, never fabricated Succeeded.
Registry input-required without an offered mirrored form exposes desktop-only
attention; stock bus/transient message/tool events project safely as activity.

Receipt-backed creation is the original bounded120-second
Workspace/Create → Workspace/ShellProbe → Relay → Launch → Ready job.
The approved shared `Topology::shell_probe_once` requires explicit local
consent, a fresh idle no-focus root and matched private nonce/PID/object/result
proof before any provider stage, preserving Herdr's configured new-pane shell.
Only source-admitted GNU Bash5.3p9 (bash/bash-interactive) and zsh5.9/5.9.1 use
the fixed shared probe; package/version admission is not live shell proof.
Launch uses only the exact Herdr kind/name/pane/args contract and requires this
action's new-root proof. Submission ACK
retains resources/Launch-Dispatching. Read-only alias/terminal/process/namespace/
registry/in-memory welcome.header/FINAL writable snapshot readiness occurs
outside write fences before confirming NativeId and the exact resulting Target;
the optional persisted header is history evidence only. No relaunch, rollback,
automatic restart replay, fabricated shell measurement or desktop-focus repair.

Frozen shared APIs for5B/6: `native_json::{parse,Limits,rpc_id,decimal}`;
`Decimal`/`NativeRpcId`; required `SelectionMarker` plus help/count fields in
both protocol consumers; `Topology::{begin_native,finish_native,shell_probe_once,
validate_provider_stage,record_shell_measurement}` with exact action/new-pane
evidence; `Security::native_guard`; existing source-scoped history/content/
receipt owners. The one-shot native install path is removed. No provider trait,
registry, second store or wire model is introduced.
`Runtime::mutate` registers the exact accepted original in `Security::reserve`'s
post-commit callback on the serialized SQLite owner, before its cancellable
reply. Validation/capacity/alias/SQL failures and matching existing receipts
never register a job. No action-ID-only status read adopts another reservation.
Newly reserved SessionCreate returns that committed Reserved receipt without a second reply
wait; the original job and native deadline continue unchanged.

Before entry readiness, `Security::preflight_request` moves the owned request
through the serialized Storage owner for authenticated canonical/HMAC receipt
matching; it makes no reservation or registration. Shared lookup logic preserves
expiry and matching-receipt-before-stale-host/Core precedence. An unseen malformed
method/boot is rejected before native waiting. Canonical buffers are zeroed on
success and failure, and the request is not cloned across the readiness await.
The authorization gate and SQLite transaction are released before that wait.

After readiness, exactly one ordinary atomic `reserve` repeats authorization and
canonical matching and registers a new original only after commit. Only an
original whose prewait succeeded may carry its captured Arc into reservation:
the one canonical Core validator checks that exact Target/source epoch under its
lock, capabilities and request/turn/answer witnesses. For OMP alone, a late
`topology_dirty` Synchronizing revision may reserve **no-effect intent**, provided
the Core is neither installing nor install-failed and the same current Arc and
native physical/source identity still hold. The matching native validation may
tolerate only typed source-pending at this no-effect boundary, not source I/O.
Offline, clean/unproven synchronization, replacement, disabled capabilities and
resolved requests remain terminal. Reserved never means Ready or write permission.
A concurrent matching receipt or UUID conflict still wins at the final lookup,
even if the intervening prewait failed; an unseen prewait failure cannot reserve.
There is no reserve retry, wait in SQL/authorization locks or changed creation path.

An unseen original and its later committed writer may wait at connection readiness
only for an explicitly absent/dirty authoritative Source revision or a
physically fenced, topology-dirty, same-core OMP `Synchronizing` state. Capture
one connection Arc, Target and source epoch; every iteration checks that exact
current mapping, native process/file/source identity, membership, capability/dialog
witnesses, paired authorization and shutdown. The canonical Core read returns its
already-validated connectivity instead of copying/re-reading a display snapshot;
installing, install-failed, source I/O, identity/mapping loss, revoke, disabled
capabilities and resolved requests are terminal. If an original strict Core check
saw Synchronizing and the existing same-epoch eligibility reread is now Ready,
that healed result is not a stale NotReady rejection or dispatch permission:
release the authorization guard and repeat fresh strict validation through the
existing100ms bounded cadence. A still-dirty Synchronizing reread uses that same
cadence; other conditions fail immediately.

Entry and common `write_once` readiness consume the same absolute120-second
deadline captured at mutation entry. The captured connection is carried through
the one post-commit registration into the writer, not reacquired by Target.
This starts no new window and does not reinterpret the
14-second reply waiter as dispatch authority. Each5-second writer acquisition is
also bounded by that remaining original deadline. The10-second native-I/O timer
starts only after actual boundary admission; the final strict Ready/native-source
validator is never relaxed. No reservation, native effect or receipt is replayed,
and no replacement connection/Target is selected while waiting.

Readiness can change after the writer prewait or while the shared fence persists
intent. For future original jobs only, OMP retains one non-clone
`UnstartedBoundary` claim for that same durable intent/ticket and socket.
An original typed source-pending/Core-sync failure may defer only after the
existing exact-epoch, physically fenced no-effect validator proves the same
current connection, dirty OMP revision, capabilities and dialog witnesses.
The unpolled write future and writer/authorization guards are dropped before
waiting through the existing readiness path. The original sealed bytes and result
stay owned in memory and are consumed only by the one actually started effect.
Each attempt reacquires the writer and fresh authorization and requires strict
Ready on the pinned Arc/Target/source epoch immediately before marking started.
This is pre-effect continuation, not another reservation, intent or native write.

Before intent the receipt remains Reserved; after durable intent it truthfully
remains Dispatching-but-unstarted, never rolls back to Reserved. Deadline,
revocation, shutdown, cancelled ownership, I/O, identity, capability or resolved
dialog failures settle the existing unstarted claim with definitive no-effect
proof. Cancellation-safe ownership is installed on the SQLite owner before its
reply; cancellation during handoff cannot orphan a claim. A crash still recovers
the durable intent as Uncertain, never a fabricated no-effect success or replay.
Once started, the existing ten-second timeout, descriptor invalidation and
lost-outcome rules apply unchanged; no started/ambiguous write is retried.

The actual run0a3eaaab returned `boundary/source_pending/NotReady` and a Failed
receipt after its original durable intent. That first condition proved no native
write was admitted; it was not a Stop/Idle pass. Its failed receipt and native
work remain retained without replay or adoption.
`unstarted_native_boundary_defers_after_intent_without_replay_or_blocking_revocation`
uses the existing NativeFixture: commit one intent, then dirty the real subscribed
source before start; assert Dispatching, zero polls and an authenticated no-effect
stream marker, then refresh and require one original abort on the same source.
Its revoke/shutdown/deadline/I/O/mapping/capability/dialog/cancellation cases deny
all native effects. These are consumer regressions, not stock-native acceptance.

OMP still means **current work at authorized dispatch**: it supplies no native
turn token or actor seal for Stop/steer, and this wait cannot select a historical
turn. Original dialog request keys/native IDs remain fixed and revalidated; no
new work-generation policy or idle-Stop capability rule is introduced.
`committed_original_waits_for_same_source_then_dispatches_once_or_fails_without_effect`
now obtains actual prewait Ready, then gates an ordinary subscribed notification
before the one final reservation. The old strict validator rejects that actual
late-dirty window; the new no-effect intent remains Reserved while an ordered
encrypted-stream marker proves no abort preceded refresh. The same Arc/Target/epoch
then dispatches one abort after refresh, while I/O/mapping loss, revoke, original
deadline expiry, shutdown, capability loss or dialog resolution produce no effect.
The narrower strict-error/healed-reread interval has no deterministic existing
fixture gate; no test hook was added. The shared canonical-mode regression covers
OMP-only dirty eligibility, exact epoch, install/failed-install, capability,
clean synchronization and Offline rejection, with public strict dispatch unchanged.
These consumer regressions are not stock-native acceptance.
`unseen_core_sync_wait_preserves_matching_receipts_and_one_final_reservation`
also exercises actual pre-reservation Core synchronization through the existing
native fixture: no reservation while pending, one authenticated write after
refresh, offline/revoke no effect, stale-host/Core matching-receipt precedence,
and concurrent canonical match/conflict without a second native write.
The historical `admission/core_mutation/NotReady` results, including run47ef4725,
identify only a Core readiness rejection: the old format did not distinguish a
stored entry failure from the final reservation validator, nor Synchronizing from
Offline/capability/install state. It does not prove that prewait succeeded in that
run. The permanent canonical Core check still distinguishes the exact dirty
Synchronizing eligibility required by the no-effect reservation and boundary
rules. That functional classification remains; temporary entry/final failure
reporting and captured I/O metadata have been removed after causal native proof.

A separate earlier owned run reached native Ready and dispatched its original
same-Target prompt, then rejected the first mobile history `session.open` as
StaleSession. A later stored generation records follower replacement but cannot
identify or date the original native/file/frame failure. It is not permission
to retarget that action or weaken incarnation checks.

The fully automated owned run `b2afd3f3-703e-439b-98d6-2446395b9236` subsequently
passed the post-desktop Stop milestone on its original Target/generation:
the two source-ordered phone prompts and exactly one `turn.cancel` have durable
Dispatched receipts with no error, and the current session ledger retained the
same native identity/generation. Entry into native history generation is gated
by the helper's already-observed same-Target Ready/Idle within the original
60-second Stop scenario deadline. This proves accepted dispatch plus that idle
observation, not a per-request OMP acknowledgment or interruption of `sleep`
before natural completion.

Temporary invalidation capture is removed. Ordinary runs retain null stderr;
the owned relay listener remains only for real isolated creation. Native
authentication, file/header checks, ErrorCode projection, original-epoch
reservation and unstarted-boundary deferral remain unchanged. The clean package
and combined OMP+Codex+Android acceptance remain coordinator-owned.

Shutdown stops admission, awaits a serialized SQLite no-op with queue-waiting
admission, then takes and joins the final native job set before returning any
barrier error. Earlier validated originals commit/register despite reply
cancellation; later validation fails NotReady. Unentered originals settle
Failed/NotReady without native bytes; desktop providers retain independent lifetimes.

Retained direct choices are aes-gcm0.10.3(aes/alloc), axum0.8.8(http1/tokio/ws)
and tokio-tungstenite/tungstenite0.28.0(connect/verified Rustls WebPKI roots);
Rustls remains0.23.45/ring, with no native/insecure TLS fallback. Exact selected
sources/checksums/features/licenses remain in Cargo.lock/dependencies.json;
RFC3339 timestamps use the already-retained chrono0.4.45 std parser (promoted
direct without a newly resolved package), never a floating-point timestamp.
NOTICE retains applicable OMP MIT terms. Dependency resolution was permitted;
build, formatter, tests, advisory audit, real native process/namespace/wrapper/
probe/relay reachability, no-focus launch and both-backend/APK acceptance require coordinator validation. Installed source/dist equality
and original host kernel/user-manager proof are not fabricated.

## Acceptance gates

See [protocol fixture parity](../ACCEPTANCE.md#protocol-fixture-parity), [live session and creation](../ACCEPTANCE.md#live-session-and-creation), [provider dialogs and cancellation](../ACCEPTANCE.md#provider-dialogs-and-cancellation), [generation replacement](../ACCEPTANCE.md#reconnect-and-generation-replacement), [mutation receipts and restart](../ACCEPTANCE.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.

## Durable observed-cycle attention

Only post-install, source-ordered live `agent_start` and `agent_end` open/close
the host-owned observed cycle. Storage atomically records original Target,
generation, private source epoch, connection token and cycle ordinal; start
checkpoints Working, and end allocates durable peer attention events plus push
jobs before live publication. Duplicate start keeps the active cycle; end
without it emits nothing. Source gap/reconnect/restart retires without completion.
Registry busy, initial/replayed snapshots, turn_end, tools and history never
complete it. Request attention uses actual original native request IDs in a
separate namespace. No extra provider read or RPC supplies a correlation.

## Enabled Skills extension channel

Fresh owned launches can append the Goatr-owned hash-pinned extension through the unchanged stock outer wrapper. This optional feature pins installed public OMP18.8.7 package/CLI/source hashes; it does not change collaboration admission or adopt existing processes. Desktop `omp_skills_package_root` defaults absent, and Herdr `terminal.mobile_skills_preview` defaults false. Both configuration and the original verified process namespace/object join are required.

The original OMP process supplies enabled `source:skill` rows through public `pi.getCommands()`. No directory scan or guessed catalogue is allowed. A private0700 launch directory contains an exclusive0600 capability record and extension; the capability never enters argv or phone frames. The private Unix channel proves SO_PEERCRED against the admitted OMP owner, exact native conversation from public sessionManager.getSessionId(), original backend Arc and native scope. Public session switch/shutdown retires the channel permanently; reconnect cannot adopt it. Pending handshakes are bounded to eight/30seconds, channels and consumed launch slots to32, requests to4096/channel, with one-hour channel expiry. Retained launch files require owned cleanup; no previous launch is adopted.

Catalogue refs expire after five minutes. Only their exact original regular objects are eligible for preview: O_PATH/type proof precedes readable opening through a verified procfs exact-FD route inside the original process. Bounded positional reads retain FD ownership until completion, checking original session/object around awaits. A complete immutable UTF-8 snapshot is at most1MiB, expires after two minutes and transfers in64KiB chunks. One snapshot per original process, one preview per authenticated peer and16 host preview records bound memory/ownership. Source, grant epoch, catalogue or process loss prevents publication. Raw documents/capabilities are never persisted or logged.

This is USER document-read authorization, independent of model tool approval. Insert adds exact `/skill:name` command text to the original scoped draft only when its name is representable. It neither expands the skill nor invokes/submits a native command. Native invocation acknowledgement remains unimplemented.

OMP document paths remain extension-local, absent from public catalogue rows and UI. The host transfers only exact command metadata and opaque refs. List/open/chunk operations perform actual native scope authorization after I/O queue admission and again after replies; exact memory close remains grant-independent. Canceled callers retain runtime-owned matching-reply drain and exact late-open cleanup, without replay or reconnect.
