# Stock Codex backend

Scope: one proven shared app-server owner/thread, native history/control/dialogs, independently supervised TUI relays and ordered desktop-visible creation.

[Documentation](../README.md) · [Protocol/state](PROTOCOL_STATE.md) · [Pairing/security](PAIRING_SECURITY.md#revocation-fence) · [Herdr integration](HERDR_INTEGRATION.md) · [Companion configuration](COMPANION_PACKAGING.md#configuration-and-trust-roots) · [Source anchors](../REFERENCES.md#critical-files--anchors)

## Baseline and live attachment

Helper private-path failures retain only a static existing check phase,
`unsafe_private_path` or `other`, and the unchanged I/O kind. A more specific
inner phase survives outer scope context; raw error text and paths are discarded.
This clarifies a failed guard without changing permissions or retrying a read/input.

Private publication reads validate the opened no-follow descriptor's owner,
regular-file type, mode, link count and size, then revalidate it and the current
pathname before accepting JSON. Atomic replacement may retire an otherwise valid
descriptor; only that retired view or a changed valid publication retries within
the existing three-attempt bound. Unsafe metadata remains terminal, and a retired
descriptor never supplies accepted presence or owner identity. Nonblocking open
prevents a substituted FIFO from stalling the regular-file check.
Exhausting those three attempts produces a typed unavailable observation, not
identity-loss evidence. That error from the presence-record phase gates the
original installed source as Synchronizing without closing its authenticated
follower or changing Target/generation. A past heartbeat may also gate an already
known original connection, but only after its connected state, owner, relay, TUI,
connection epoch, thread membership and source proofs match. Past presence never
grants live authority; public presence validation stays strict. Native events remain
ordered in the gated reducer. A serialized fresh complete
presence/owner/source/topology observation clears the mutation gate, with freshness
checked again at the completed decision. Future timestamps, unsafe files, changed
identity and reader loss remain terminal. No read attempt is added.
Creation readiness and helper pre/post-input scopes use their existing observation
cadence and original deadlines for these typed conditions. Already-pinned creation
waits for that same connection without repeating launch or Ready stages. They never
repeat an input; unavailable post-input proof at deadline remains uncertain.

Use stock Codex0.162.1 and local Unix-WebSocket app-server endpoints in
`companion/src/backends/codex.rs`, implementing the pinned native contracts
independently. Historical AgentWire Python references
`src/agentwire/backends/codex.py` and `src/agentwire/stack.py` inform
method-handling/shared-runtime behavior, not lossy projections or blind resume.
Those paths are provenance, not Goatr runtime modules. The
[public pinned reference](../REFERENCES.md#critical-files--anchors) is reachable
but not bundled; its full applicable license/copyright notice remains
unavailable, so copy no AgentWire code. Preserve installed wrappers/sandbox and
signing policy; never invoke a raw npm binary to bypass them.

Wrapper packaging must distinguish the execution-owning app-server/local role
from the remote-resume TUI client. Pinned Codex rejects remote-resume CLI
permission overrides, including sandbox_workspace_write and permissions maps.
A complete private wrapper-chain correction may omit only its own added local
execution-permission arguments for the supported remote-client invocation.
Preserve every caller argument so explicit caller overrides still reject, and
preserve the outer sandbox, namespaces, signing environment/mounts, original
native executable and all owner/local permission arguments. The remote client
inherits the original server-owned thread policy. Build/pin normal executable
provenance; never invoke the ELF directly or alter global config/trust. Runtime
compatibility remains a separate actual native gate.

Initialize with `{clientInfo:{name:"goatr",title:"Goatr",version:<version>},capabilities:{experimentalApi:true,requestAttestation:false,optOutNotificationMethods:[]}}`, then `initialized`. Follow every page of `thread/loaded/list`. Attach only to a proven shared owner/thread, calling `thread/resume` without cwd/model/policy overrides, `excludeTurns:true`, `initialTurnsPage:{limit:20,sortDirection:"desc",itemsView:"full"}`. Keep the subscription while the desktop binding exists. Stock resume has no atomic requireLoaded guard; if owner/thread presence changes around attachment, report the changed/cold state and never describe it as an untouched live attachment. Embedded/unproven owners show `Shared mode required` with explicit setup instructions; no automatic exit/resume.

A typed private terminal-proof availability is required on every relay presence record; missing values fail closed. During an active same-scope bridge only, an exact snapshot or process_info BrokenPipe publishes unavailable while retaining the proven binding, TUI, epoch and associations. Native observer traffic cannot clear that gate. The normal heartbeat must complete the same-scope terminal proof before it publishes available; closure, source replacement, malformed proof and reader loss remain terminal. Backend reads map unavailable to the existing Synchronizing/observation-pending gate, and mutation authority remains unavailable.

A loaded thread is necessary but not sufficient: require a validated relay binding proving this Herdr terminal's TUI successfully associated that thread on this exact owner epoch. Pre-existing shared TUIs qualify only when launched through this helper and still backed by that proof (including manifests surviving companion restart); an arbitrary known app-server socket or AgentWire presence file is not enough. Embedded/unproven panes remain visible UnboundRows. Check owner/binding/loaded membership before and after resume; the non-atomic stock loaded/resume race remains disclosed, never patched with a fictional `requireLoaded` option.

An already-followed session's read-only timer uses the same `read_connection`
validation as reads. A topology-refresh `NotReady` retains that follower only
when its exact target and physical owner/relay/TUI/thread/source proofs still
pass; mutation admission remains strict. Real relay connection-epoch, native
owner/process/socket/incarnation/thread changes or EOF still invalidate the
binding and require generation replacement. Old targets are never retargeted.

Creation readiness distinguishes an absent/dirty authoritative Source revision
from a failed physical check. It pins the first matching installed connection and
Target; only that connection's source-refresh or reducer-synchronizing state may
wait within the original absolute300-second creation deadline. Owner, publisher,
TUI, thread, current connection, generation, membership, focus and authorization
checks still run; I/O, identity loss and replacement remain terminal errors.
The public mutation validator stays strict.

Launch confirmation and Ready intent are written once. Refreshes encountered
after those Storage awaits wait for the same pinned connection without repeating
stages or selecting another Target. Final Ready requires fresh source/root proofs,
and deadline checks cover the existing awaits, drain/wait boundary and queued
Ready writes. No readiness wait replays a native operation or extends the budget.

Returned errors during the entered Thread stage retain only a closed phase and
existing ErrorCode in their Uncertain proof. The acknowledged native ID remains
in its original receipt resources and is never included in that diagnostic text.
These details grant no success, replay or recovery authority. An outer boundary
timeout/cancellation remains unclassified unless its existing settlement records
the actual initiating condition; missing proof never establishes a timer failure.

Owned Codex diagnostics also retain the existing shell query's closed resolution
rejection before Workspace receipt projection. The already-classified query drop/rejection, result wait/authorization/shell/read/
validation and final reproof failures also use the same Codex prebinding latch
and eight-line counter, before error projection. Existing guard lexical scopes,
errors and return order are preserved. Ordinary runs and OMP do not enter this capture; no query,
read, deadline or admission rule changes. Remove it after exercising the causal fix.

The same finite reporter captures the first pre-Thread creation return after
confirmed Owner: deadline before reproof, existing owner-reproof error, deadline
after reproof, or stopping. These observations precede receipt projection and
preserve the original error and short-circuit ordering. They do not infer the
initiator or ordering of a later transport close.

The same owned-run reporter has an independent first transport-cause latch,
armed only by the original authenticated Codex creation connection. Reader,
output and supervisor terminal observations precede teardown and retain only
closed Method/phase/guard, optional actual ErrorCode, frame stage and bounded
I/O kind/errno. Every selected supervisor winner is captured before stop, including
success and distinct signal branches. Native latches remain separate; all reporting shares
the eight-line limit. Ordinary runs and OMP stay silent. No read, request,
probe or deadline is added. Remove this temporary capture after causal exercise.

The same protected annotation may retain one capture-only snapshot of the first
expected-thread `thread/read` with `includeTurns` absent (native defaultfalse)
or literalfalse: request/reply category,
both original send completions, matching resume request seen and first correlated
RPC error family. Correlation IDs stay private. Existing parsed frames and
charged queue entries supply these facts; no duplicate parse/read/RPC or line.
An independent short synchronous lock freezes the snapshot at winner selection
before teardown; it spans no await and ignores later updates. The annotation
stays below512bytes. Completed send proves transport completion, not native
client deserialization or acceptance.
The same frozen annotation records the first admitted `account/read`, its
result/error category and both original send completions. At the existing
correlated result it also classifies only object shape, account.type and
requiresOpenaiAuth into closed account-shape/auth-required tokens. Missing/null,
known account types, invalid and unknown-type shapes remain distinct; no account
strings, unknown values or credentials are retained. Classification cannot
change forwarding. These fields share the existing winner-frozen annotation
and add no line, read, RPC, timer or authority. They describe metadata, not native
typed decoding/account acceptance or the cause of closure.
The same annotation may capture the first admitted model/list and
configRequirements/read lifecycle as two closed five-bit masks: request seen,
request send completed, correlated result/error received, and reply send
completed. Existing parsed metadata/correlation and charged queue markers supply
these facts; no native payload inspection, additional request/read/probe/timer or
report line. First-error categories remain closed; ordinary native error winner
and publication ordering are unchanged. Freeze at the existing selected winner,
ignore later updates and retain the existing below512-byte annotation bound.
Delivery completion alone does not prove native client decoding or acceptance.
For a captured generic admission failure, the same existing final annotation may
retain initial Ready publication completion time. It is sampled immediately after
the original successful publication, preserves earlier errors and grants no
readiness/deadline authority. Ordinary runs retain their existing message.

The separately selected owned startup Terminal check uses the reviewed native
read capability. An immutable prelaunch pin comes from an existing validated
snapshot. After original creation settles, one inventory and one atomic bounded
read must match that original runtime, source, manifest and receipt resources.
Binding replacement rejects without retry. Only a closed visible-signature
category survives in memory; no terminal text, hash or screenshot is retained.
The existing joined drain emits it within the same eight-line budget while
preserving earlier evidence, incomplete flags and the original operation error.
It does not prove a native error object or phone Terminal acceptance.

On serve shutdown, signal stop and await every non-selected admin, transport and
topology future once in their existing order before propagating sibling errors.
Transport owns its provider drains. Preserve admin→transport→topology→selected
error precedence, then perform the existing security/endpoint/storage cleanup.
A sibling error cannot skip those drains; a failure inside later cleanup retains
its existing error behavior. Runtime evidence belongs in [Status](../STATUS.md).

Temporary owned-run diagnostics classify the first terminal Owner validation
return before generic projection, including service provenance, native process,
socket epoch and initialization. They use the existing relay-gated Codex reporter
and shared eight-line limit. Closed guard/ErrorCode/I/O categories retain no
service output or native contents; retries, reads, authorization lock lifetimes
and deadlines remain unchanged. Remove this capture after exercising the causal
fix. Runtime status belongs in [Status](../STATUS.md).
Owner/relay startup keeps its existing five-second child-status polling deadline.
After the existing kill, the exact owned child is reaped on Tokio's blocking pool,
so waiting for it cannot occupy an endpoint worker. Cancellation leaves that
reaper owning the child; the original Timeout precedence and start/receipt fences
remain unchanged. This alone does not identify a captured connection timeout's cause.
Owner native-process attestation also runs on the blocking pool: its existing
pure executable proof can hash the full pinned distribution on a cold cache.
The original creation deadline bounds spawning, waiting and accepting the result.
Join failure is terminal; cancellation leaves only read-only proof work. Subsequent
process, socket, authorization and topology proofs remain required before effects.

Owner-to-TUI forwarding is passive once one original frame is admitted: it has
no independent five-second send timeout and keeps that frame's existing
queued/in-flight permits charged until the same send completes or the bridge's
existing terminal condition closes it. Pending delivery never reports success,
renews authority or recreates/resends a frame. The reverse TUI-to-owner write
keeps its existing five-second deadline and terminal cleanup.

Only the validated owned Codex capture retains the winning forwarding worker's
closed termination category in the existing protected presence error: TUI or
owner EOF/Close, or the corresponding drained writer queue. Ordinary runs retain
the generic forwarding-closed message. Existing failures and first-error
preservation take precedence; the selected winner does not prove which peer
initiated a close or that an aborted sibling had no error. No additional read,
RPC, timer or output line is introduced. Remove this distinction with temporary
capture after causal exercise.

The same finite reporter records the last closed Thread effect phase only if
that original effect is dropped. An already-retained reader failure takes
precedence. This uses existing in-memory state, adds no native observation and
does not classify cancellation as a timeout or grant reconciliation authority.

For the original thread/start callback, the drop observation can retain only
closed `unseen`, `matched`, `delivered` or `receiver_closed` milestones,
decoded/unmatched-frame booleans and an actual closed result code. `delivered`
means the original oneshot accepted its result, not that the awaiting effect
consumed it. The callback window closes before teardown; late events cannot
change the frozen snapshot. The start-reply drop tuple additionally retains a
closed original-send elapsed bucket relative to the existing boundary budget,
and whether a complete data message was observed in that callback window.
Elapsed time begins at effect entry, not the exact outer timeout origin. A complete
data message is not necessarily the correlated reply or proof of native admission.
Decoded/matched observations imply a complete data message under the same lock,
including callback-activation races. Title-reply drops do not use this start window.
The same snapshot retains bounded before/after masks of eight fixed method
families. Known notifications and server requests require their exact native
shape; other names retain only the corresponding other family. The existing
send-completion marker and frame classification share one lock. These are
observation-order categories, not socket order, request correlation or admission.
The helper accepts canonical masks0..255 and exposes only fixed family names.
Private tuple keys are compact so every valid line, including newline, is at
most230bytes under the unchanged256-byte reader bound; saved JSON field names
remain descriptive. Oversized/rejected lines make the capture incomplete.
No additional output line, native read, parse or timer is added. Ordinary runs and OMP remain silent.

The same owned-run reporter may emit three non-failure Owner control-flow
milestones: original start settlement returned, child status returned, and entry
into native initialization after the existing proofs. It privately pins the first
fresh Owner action and deduplicates those milestones, admitting them only while
the shared count is below three. At least five of eight slots remain for failures;
progress never consumes failure latches and stops after one wins. Exact closed
`code=none io=none errno=0` tuples imply no native success or readiness. There
are no added reads, RPCs, awaits or timers. Missing milestones require a complete
drain and intact budget before supporting an inference about control flow.

The helper retains finite capture progress independently of the drain thread
handle, including on incomplete cleanup. Worker EOF/read failure/panic and
cleanup outcome remain separate closed observations. A partial snapshot never
claims EOF, absence of later output or the original native cause. No lock spans
I/O, joining or awaiting; the existing cleanup deadline and error precedence
remain. This is temporary diagnostic behavior, not native readiness authority.

The final shell proof exposes its existing classified guard before projecting the
same error code; shell absence/busyness, reporter/process/executable failure and
reporter recheck remain distinct finite categories. It adds no native read and
does not identify ordinary shell busyness as replacement. The explicit Codex-only
helper diagnostic can append the original connection's finite close family to
its existing frame-read failure before cleanup. An `open` family means no
connection-level reason was available then, not proof of a usable stream.

Owned relay startup capture is armed only after the existing manifest validation
and exact per-binding diagnostic opt-in. The existing service launch explicitly
routes diagnostic stderr to the user journal. A one-shot closed report observes
an existing pre-Ready failure before error projection; it preserves the original
I/O error, call order, return, deadlines and authorization decisions. Output must
not panic or expose the raw error, including if the sink fails. Ordinary runs and
OMP retain their existing behavior; failure before validated opt-in is unavailable.

After original operation settlement, stop, stderr join and terminal observation,
the selected helper may query that exact owned service journal once within the
remaining cleanup budget. Join retained action/Relay-stage/unit/binding/manifest,
original boot/source and executable identity without new native validation calls.
Bound the read to100 records/1MiB and admit only the closed producer vocabulary
through the existing shared eight-observation budget. Trusted service timestamps
must fall between receipt creation and original operation settlement; later
teardown failures cannot explain the initiating cause. Missing, ambiguous, late,
exhausted or unjoinable capture is unavailable. Journal availability is separate
from pipe EOF; it never replaces the original operation error or grants readiness.
Remove temporary capture after exercising the causal fix.

## Native history, prompts and cancellation

The guarded thread/start/read result declares thread.historyMode and its exact
thread.id. Paginated mode uses thread/items/list with threadId, limit100,
sortDirection desc and **no turnId**. Native opaque cursors traverse the flat
thread item store in creation order. Missing/unknown modes fail capability
admission; a timeout never selects a different reader. Explicit legacy mode
uses the bounded turns walker, one turns page or fallback item page per advance.

The latest snapshot reads one native head page. Browsing starts a separate native
head view, independent of the live1000-row cache and its arrival ranks.
Each browse view follows the native opaque successor past the live1000-row cache
while Android retains only one bounded local window. It makes no unlimited native
retention claim: the native terminal cursor, fixed expiry or explicit capacity
error ends traversal. The shared [browse contract](PROTOCOL_STATE.md#native-browse-views)
owns tickets, fixed expiry, coalescing, frozen membership, byte/RPC limits and
explicit errors.
Paginated incomplete items consume positions but never become SafeHistory.
Cross-page item/cursor duplicates fail closed. No retained-boundary head seek,
turn hydration sweep, lexicographic cursor arithmetic or cache pagination
supplies native paging.

Native item/started and delta notifications update bounded in-progress state and
Activity. Only item/completed or native completedAtMs produces completed rows;
live item IDs and literal native turn IDs remain unchanged. Initial/reconnect
state and native controls use the original thread, owner and Target. The normal
live stream remains active during browsing; desktop selection is still unknown.

`prompt.submit` always calls `turn/start {threadId,clientUserMessageId:<action UUID>,input:[{type:"text",text,text_elements:[]}]}`; its stock busy behavior may steer, which the composer labels. Do not add a separate steer/settings API in this scope. Cancel calls `turn/interrupt {threadId,turnId}` with the observed native turn ID and rejects stale turns. Neither path uses PTY input.

## Native approvals and questions

Use protocol finite presentations and saved requestKey→native-ID mappings; preserve integer versus string native JSON-RPC IDs exactly. Send a JSON-RPC result to the **original request ID**, never a fabricated resolve method. This is a bounded renderer of the pinned runtime contract, including experimental fields enabled at initialization; stable generated schemas alone omit part of that contract:

| Native server request | Inputs/presentation | Native result and permitted choices |
|---|---|---|
| `item/commandExecution/requestApproval` | CommandExecutionRequestApprovalParams: threadId/turnId/itemId, distinct approvalId when present, kind command/writeStdin, command/actions/cwd/reason, network context, proposed amendments, experimental availableDecisions and additionalPermissions; `codex_approval` | `{decision:"accept"\|"acceptForSession"\|"decline"\|"cancel"}` or exact offered `{decision:{acceptWithExecpolicyAmendment:{execpolicy_amendment:[...]}}}` / `{decision:{applyNetworkPolicyAmendment:{network_policy_amendment:{host,action:"allow"\|"deny"}}}}`. These are possible shapes, not universally offered choices: apply the runtime membership/default rules below. Show complete requested permissions and session/persistent-policy scope before selection. |
| `item/fileChange/requestApproval` | FileChangeRequestApprovalParams: thread/turn/item, reason, grantRoot if supplied; correlate file changes from that exact native item, mark absent details unavailable | `{decision:"accept"\|"acceptForSession"\|"decline"\|"cancel"}`. Do not promise grantRoot enforcement: pinned schema itself calls it unstable/uncertain. |
| `item/permissions/requestApproval` | PermissionsRequestApprovalParams: exact requested network/fileSystem profile, cwd/reason; render structured paths/globs/special roots and read/write/deny scope in details | Choices are deny `{permissions:{},scope:"turn"}`, allow exact requested profile with `scope:"turn"` or explicit `scope:"session"`. Never broaden/edit paths or network scope. Pinned response permits optional strictAutoReview; omit it, do not silently disable review. |
| `item/tool/requestUserInput` | ToolRequestUserInputParams: isBlocking and question id/header/question/options descriptions/isOther/isSecret; `codex_questions` | `{answers:{<questionId>:{answers:[<strings>]}}}`. Preserve IDs and option labels, support free text when options absent and Other only when offered; no general native cancel invented. |
| `mcpServer/elicitation/request`, mode `form` | McpServerElicitationRequestParams: serverName/threadId/optional turnId/message/requestedSchema; `codex_form` | `{action:"accept",content:{<field>:<typed native value>}}` or `{action:"decline"\|"cancel"}`; required action, content omitted for nonaccept; omit optional response `_meta`, do not echo request metadata blindly. |
| same method, mode `url` | message/url/elicitationId; `codex_url`; only explicit HTTP(S) navigation | `{action:"accept"\|"decline"\|"cancel"}`; opening browser alone is not completion or proof of native resolution. |

For command approvals, a present `availableDecisions` is the authoritative ordered list: preserve each exact native value, including amendment payloads, in the in-memory choice map and validate answers against that request's saved list. Never add base choices or choices inferred from separate proposals. An empty list offers no mobile answer; malformed/unknown decisions or unrepresentable scope make the request `desktop_only`, not a filtered actionable subset.

Only when `availableDecisions` is absent, use the pinned `ExecApprovalRequestEvent::default_available_decisions` order, mapped through `CommandExecutionApprovalDecision::from`:
- With network approval context: `accept`, `acceptForSession`, the **first** proposed network amendment whose action is `allow` if any, then `cancel`. Do not add proposed deny amendments.
- Otherwise, with additional permissions: `accept`, `cancel`.
- Otherwise: `accept`, the exact proposed execpolicy amendment if any, then `cancel`.

These defaults never add `decline`; only the network-context branch supplies a default session grant. Present lists take precedence over every fallback branch. Render **all** `additionalPermissions` fields and requested network/filesystem scope in approval details without dropping, broadening or editing them; unsupported/unknown/oversized permission details make the entire request `desktop_only`. The existing finite choices/details protocol suffices; the phone never supplies native permission/amendment JSON. See [runtime definitions, producer and defaults](../REFERENCES.md#critical-files--anchors).

Standard MCP form support is precisely object properties of string (email/uri/date/date-time, minLength/maxLength/default), number/integer (minimum/maximum/default), boolean, titled/untitled single enum, legacy enumNames and titled/untitled multi enum (minItems/maxItems/default), plus root required. Preserve titles/descriptions, exact enum values and validated defaults. Normalize native finite numeric tokens to protocol decimal strings without Float/Double round trips; reject unsupported bounds/types rather than weaken validation. Unknown constraints, nested/proprietary schemas, `openai/form`, `openaiForm`, `openai/userVerification`, dynamic tool execution and connection-owned attestation remain desktop-only; no synthetic biometrics. Never cache raw pending originals or secret answers.

Ordinary native requests are first-response-wins; `serverRequest/resolved` closes cards but does not acknowledge which client won. Reconnect to the same proven owner, replace generation, install current pending replay/history and never resend previous answers. Source schemas establish shapes, **not** replay/order/busy/concurrency behavior; required real-owner checks cover loaded-list pagination, resume-before-notification ordering, pending replay/first response, turn/item pagination and busy turn/start. AgentWire's input/approval projections are partial references, not conformance.

## Independent owner and relay services

Provide independent `goatr-codex.service` for the stock app-server: `Type=exec`, `UMask=0077`, configured wrapper-accessible working directory and the same packaged binary’s hidden `codex-owner --record PATH` entry. Before verification, subprocesses or runtime threads, that entry clears inheritable, permitted, effective and ambient capabilities, then execs the unchanged absolute installed Codex wrapper with `app-server --listen unix://<socket_agent>`. No global capability policy or sandbox/signing override is involved. Both owner and relay use `systemd-run --user --expand-environment=no`: shell-free argv is literal at the user-manager consumer too, so `${NAME}` or dollar-bearing configured paths are never expanded. Derive the default short `c.sock` and host/provider-visible pair from configuration. No PartOf/BindsTo/shutdown tie to companion, auto-updating daemon or new OpenAI remote-control service. Start only on explicit setup/new creation if absent, never to recover an old live session. Type=exec proves exec only: require socket owner/mode/process epoch plus native initialize handshake before owner-ready.

User-manager proof uses the unchanged exact `/run/user/<uid>` private0700 root,
then a canonical same-UID `systemd` directory without group/other write access
(native0755 is valid) and native `systemd/private` Unix socket0700. Systemd260.4
[binds that socket under umask0077](https://github.com/systemd/systemd/blob/v260.4/src/core/dbus.c#L984-L1029).
Recheck its device/inode before and after connecting, same-UID `SO_PEERCRED` and
the existing actual kernel PID/start/namespace proof; socket permissions alone
are not manager readiness. This manager-only policy does not change Codex
physical-owner or direct-relay parent0700/socket0600 validation. The consumer regression
`manager_native_socket_permissions_admit_creation_without_weakening_codex_sockets`
covers native manager modes and rejects writable, symlinked and non-socket
objects. Actual owned creation evidence is recorded below; the fixture is not that proof.

`companion/src/codex_relay.rs` independently implements Unix-WebSocket forwarding from the native contracts; no AgentWire code is copied. Internal `codex-relay --binding PATH` runs the **same packaged binary** in an independent `systemd-run --user --collect --expand-environment=no --unit=goatr-codex-relay-<full binding UUID>.service --property=Type=exec --property=UMask=0077 --property=Restart=no <absolute packaged goatr> codex-relay --binding <manifest>` service. Retain that same fully qualified `.service` name through collision checking, creation metadata, user-manager service lookup and exact typed D-Bus `Manager.GetUnit`/`ExecStart` inspection; do not rely on systemctl suffix normalization at an exact typed consumer. No scope/pty/pipe, serve child, PartOf/BindsTo or restart propagation. Parent0700/socket0600; both path views≤107 UTF-8 bytes; compact `t<12 random hex>.sock` with exclusive pre-admission collision handling. Origin/nonupgrade/second TUI rejected. Forward original Text/Binary payloads unchanged, with bounded observation only: frame128MiB, queued **and in-flight**32 frames/256MiB per direction; overflow fails closed without recovery writes.

The opaque signed-TUI relay uses the pinned stock client's
[128-MiB receive ceiling for both WebSocket frames and messages](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-client/src/remote.rs#L67),
applied by its [remote WebSocket configuration](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-client/src/remote.rs#L814-L817).
Each direction retains32 queued/inflight frames and two maximum-message slots:
a finite256-MiB weighted byte/write-buffer budget, with no preallocation.
The observer validates whole-message UTF-8/JSON syntax, the stock depth128
recursion budget and escaped Unicode surrogate pairs with an allocation-free
lexical guard. Borrowed `RawValue` metadata views require actual objects,
preserve absent versus explicit null, and reject decoded/escaped duplicate keys
in each observed envelope/control metadata object. Unrelated opaque params and
result data are not decoded into a full tree or charged against Goatr's
131,072-member native API budget. Exact raw RPC tokens (including `-0`), original
pending-ID consumption, thread substitution/unsubscribe and association limits
remain fenced. Messages move through the queue without copying or
reserialization. Goatr's separate native API Link keeps its
8-MiB WebSocket/JSON and16-MiB write/event budgets; the same held-socket owner
connector receives its explicit transport configuration rather than globally
widening phone/API limits.

`opaque_stock_message_reaches_peer_without_widening_api_budget` exercises real
Unix WebSocket messages larger than8MiB and containing more than131,072 unrelated
opaque elements through the observer and weighted queue to a native peer, with
exact raw-ID/original-thread association and byte equality. Array-shaped or
ambiguous control metadata, escaped duplicate keys, null boundaries, malformed
opaque JSON/UTF-8/surrogates and excessive depth fail closed; a consumed malformed
reply cannot later bind on replay. The API's8-MiB receiver still rejects the
large message, and a frame header over128MiB is rejected without allocating its
oversized body. This is a transport-boundary regression, not stock-native
phone/Ready acceptance.

Stock0.162.1's [Unix listener](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server-transport/src/transport/unix_socket.rs)
publishes the configured `--listen` pathname as an alias to
`/tmp/codex-daemon-<uid>/<SHA256(canonical advertised agent pathname)>`, with
private0700 physical parent and socket0600. The [fixed native directory](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/uds/src/daemon_directory.rs)
does not follow HOME/TMPDIR/profile settings; a preserved wrapper may give it
a private mount view. Keep advertised host/agent paths explicit and ≤107 bytes.
Accept only the exact same-UID native alias shape in configuration as attempt
eligibility, never native Ready; arbitrary aliases and direct relay symlinks
remain rejected.

Fresh owner proof starts from this original action's exact independently owned
unit: unchanged manager/service epochs, typed outer argv and environment digest,
then a unique publisher-proven native descendant ending with the exact recorded
app-server argv, preserving the installed wrapper's preceding native settings.
Extra trailing options or another endpoint do not match this original launch.
Do not infer a PID from an alias, globally scan, adopt a foreign/provisional unit
or start another owner. Resolve the advertised parent/alias and private physical
parent/socket using a held evidenced process-root descriptor and
`openat2(RESOLVE_IN_ROOT|RESOLVE_NO_MAGICLINKS)`; canonical parents and physical
paths additionally reject symlink components. Host/provider object joins,
same-UID/modes, deterministic target, root/alias/parent/socket identities and
actual native peer/publisher epochs must hold before and after connection.
Connect through the caller-held verified socket descriptor reference, not a
textual `/proc/<pid>/root/...` path. No unproven optional owner epoch remains.
Owner-process discovery shares the remaining original absolute300-second creation
budget; it does not impose a separate10-second wrapper-startup window. Native
initialization is bounded by that same deadline, late success is closed before
confirmation, and Thread admission rechecks expiry after intervening waits.
Only successful native initialization completes the saved reusable owner record;
old Owner/Uncertain provisional records remain nonadoptable.

Codex SessionCreate calculates that300-second deadline once before reservation.
It covers the whole original job without resetting between stages. The initial
durable receipt returns immediately; action.status observes its later progress.
The entered Thread stage uses that same absolute creation deadline, covering
start/reply, durable acknowledgement, owner reproof, title/reply and materialization.
Runtime stop cancels it before creation-job draining. Ready stop and deadline
branches take priority over the original effect; late completion is rejected.
Interruption invalidates the original descriptor and settles its unique ticket
without replay or loss of an already-checkpointed native identity. Ordinary
ten-second operation boundaries, individual socket/write limits and the
ten-second revocation fence remain unchanged. The smoke consumer observes only this
creation receipt for305 seconds; ordinary settlement remains125 seconds.

Typed `busctl --json=short get-property` exposes the property value directly:
`ExecStart` type `a(sasbttttuii)` has `data:[<one ten-field command struct>]`,
and `Environment` type `as` has `data:[<environment strings>]`. This differs
from `call GetUnit`, whose single object-path argument is `data[0]`.
The service consumers require the exact property type, one full command,
exact outer path/argv and false ignore-failure; environment parsing rejects
duplicate keys and compares the unchanged complete digest. Former extra array
envelopes are rejected, not retained as aliases or accepted through flattening.


The real-file/Unix-peer consumer
`native_rendezvous_held_socket_keeps_real_peer_and_rejects_alias_source_replacement`
also rejects replaced aliases/parents/physical sockets and insecure modes with
the original peer still unchanged. It is not stock-native Ready or zero-turn
TUI-resume acceptance; the actual scoped runtime checkpoint is recorded below.

### Manifest and presence contract

Two version1 private JSON files under configured bindings_dir, named by binding UUID: immutable launch manifest and helper-owned atomic presence. Both reject unknown version/unsafe owner/mode/symlink/oversize (>64 KiB), written via0600 temporary file+fsync+rename+directory fsync. Read verifies before/after process/socket evidence; stale files are not live proof. Types use protocol UUID/Id conventions, native process ticks are decimal strings if outside its Uint range.

- Manifest `{v:1,bindingId,ownerId,upstreamHost,upstreamAgent,listenHost,listenAgent,ownerEpoch,herdr:{serverId,incarnation,workspaceId,terminalId,paneId},expectedThreadId,createdAt}`.
- `ownerEpoch={linuxBootId,pid,startTicks,socketDevice,socketInode,executableSha256,socketSource}` uses decimal-string ticks/socket numbers and actual socket-peer/kernel/process-root evidence. `socketSource` contains decimal-string `[device,inode]` pairs named `root`, `hostParent`, `hostEntry`, `agentParent`, `alias`, `physicalParent`; these required native source anchors fence ordinary rechecks and companion restart, not just one connection. Missing old anchors fail closed without a compatibility shim. The executable digest must equal the reviewed publisher distribution below, never a self-declared version/source assertion or unit-active flag.
- Presence includes `relay`/optional `tui` process stamps with namespace/NSpid evidence and decimal-string ticks, `connectionEpoch`, finite state, `associatedThreadIds`, `resumedThreadIds`, optional `resumeRejectedThread`, `updatedAt` and safe `error`. Successful TUI start/resume/fork alone adds association; only an exact successful resume proves launch T. A definitive pre-admission resume rejection is retained separately through disconnect, not mistaken for Ready. One-second proof-checked publication and bounded freshness prevent an old connected file from authorizing controls after helper suspension/loss.

Private presence also carries optional finite `collaborationMode` observation
(`mode: default|plan`, source revision, and whether it came from a settings
notification) and optional `collaborationDefaultPlan` catalogue evidence.
These are helper observation metadata, **not Ready or mutation authority**, and
introduce no public Goatr/Android mode API. Only the latest correlated successful
TUI resume of the manifest's expected thread establishes current mode; same-thread
`thread/settings/updated` can update it. Uncorrelated/consumed/late replies or
another thread cannot supply it. A TUI thread transition clears it immediately,
and connection/source clearing removes mode and catalogue together. Unknown or
missing native mode remains absent, never Default; ordinary resume still retains
its original success semantics.

Catalogue evidence consumes the already-flowing correlated
`collaborationMode/list` response and recognizes only the actual distinct
Default/Plan pair, not names or inferred presets. Borrowed metadata parsing retains
no model, reasoning-effort, developer-instruction or configuration siblings.
Existing64-correlation, bridge-size/parser and publication/freshness limits remain
unchanged. The native source definitions are
[resume mode](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L466-L467),
[settings notification](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L330-L333),
and [catalogue masks/data](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-protocol/src/protocol/v2/collaboration_mode.rs#L15-L45).
The owned automation uses these original observations to restore only mode kind
after a real native question; it neither resumes again to discover mode nor
claims restoration of an arbitrary full settings object.


The relay enforces its existing post-load10-second readiness budget as one
absolute deadline across owner connection/close, manifest/terminal proof, listener
setup and initial publication admission. Connect and handshake retain their
individual5-second limits clipped to that deadline. Pure synchronous proofs run
on blocking workers; late results cannot authorize the next effect. Initial
Ready publication checks expiry immediately before atomic visibility and tracks
the exact visible object even after partial failure. A held filesystem-path
descriptor guards listener cleanup across cancellation without confusing the
kernel socket with its pathname. After startup, normal binding/relay checks
continue to protect Failed publication through heartbeat inode replacements.
Initial manifest loading and publisher attestation precede this clock. Creation polls the protected presence file, not a fake TUI connection, within the remaining **original absolute300-second creation deadline**; it does not start a second10-second window after service exec. Stopping, paired authorization and original source/action fences are checked before polling, after service-property waits and before accepting Ready or proceeding to TUI launch. Expired, absent or invalid readiness never authorizes a TUI launch. Initial ready/no-TUI waits at most120 seconds then exits visibly without deleting desktop resources. On TUI/upstream disconnect atomically clear live associated set and publish disconnected; allow the same still-proven terminal to reconnect within30 seconds, then exit. Owner epoch change fails the helper; it never reconnects to a replacement owner as the old session. Confirmed terminal closure publishes terminal_closed and exits; mere companion absence/topology timeout is not closure proof. No automatic systemd restart (`Restart=no`); failed/exited binding needs explicit reconciliation, not automatic TUI relaunch. Unlink only this helper's proven listener/presence resources, retain manifest/error for diagnosis.

The same proof-checked one-second Ready/disconnected heartbeat remains active
after accepting a socket, throughout the **single** pending TUI admission.
The existing owner/terminal checks still precede every heartbeat while the
admission's own checks and handshakes remain pending. Initial TUI publisher
verification runs on Tokio's blocking worker without changing its before/after
kernel, executable or object checks.

Publisher caching retains **verified file content only**, never process/start,
namespace or process-root authority. A hit requires the same registered path,
device/inode/length, mtime and ctime with nanoseconds, security attributes and
unchanged metadata on the retained executable descriptor. Cache lookup and
eviction perform no filesystem I/O while holding the cache lock; the existing
64-entry bound remains. Every invocation still freshly checks its process and
kernel/host/process-root executable joins before lookup and all held/proc/host/
root/process after-checks on both hit and miss. A stale cached descriptor is a
miss; the original ELF-magic/full-digest path remains unchanged on a miss.

The pinned native remote client has a
[10-second connection/upgrade deadline](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server-client/src/remote.rs#L65-L66)
already running while Goatr proves the TUI before its WebSocket upgrade.
An owner-verified identical held executable may therefore supply the TUI's
content proof without rehashing the whole file solely because its PID differs;
fresh TUI authority is never inherited from that owner or cached process.
`held_content_cache_reuses_only_unchanged_file_not_process_authority` checks real
held-file reuse, independent stale-process rejection, and same-length mutation
with restored mtime invalidating through ctime. It is not native admission proof.

No TUI/association or Connected state is published until admission completes;
source failure, terminal closure, cancellation or the original120s/30s deadline
drops the pending admission instead of replaying it. The existing5s age and1s
future-time rejection bounds are unchanged.
Relay manifest validation runs on blocking workers outside the publication
mutex. The shared observer stays serialized through validation and publication,
preserving opposite-direction association and mode ordering. Before publishing,
the original binding, owner, relay, TUI, connection epoch and state must still
match. Worker completion cancels a pending heartbeat; late proof cannot renew
a closed or replaced connection. Pending terminal proof does not renew presence.
The existing timestamp checks distinguish old and future records through finite
categories without retaining timestamp values or changing either bound.

The same absolute deadline stays selectable while a checked heartbeat awaits;
expiry wins when both expiry and admission are ready. It is also rechecked after
proof/lock waits, before Ready or Connected publication and before returning an
admission result, so a delayed task cannot renew or admit after its window.

`codex_relay::tests::admission_heartbeat_keeps_ready_fresh_and_cancels_failed_admission`
holds blocking admission beyond5s while consuming actual protected Ready
publications, then performs a real Unix-WebSocket upgrade. It checks empty
associations, unchanged epoch, one admission, deadline socket cleanup, a checked
heartbeat held across expiry with no renewal, an already-expired ready admission
that is never polled, and foreign-publication rejection with no late renewal.
Its inert metadata is not accepted as stock publisher/native Ready proof.
Current execution and fresh signed-TUI evidence belong in [status](../STATUS.md).

## Ordered creation and authoritative presence

Creation follows [CreationProgress](PROTOCOL_STATE.md#ordered-creation-progress): (1) no-focus workspace/root pane/terminal; (2) ensure configured owner ready, recording whether newly started, then `thread/start {cwd:<project>,serviceName:"goatr"}` and retain returned thread.id T plus subscription without sending a turn; (3) create manifest/start independent relay/await ready; (4) after verified Herdr wrapper resolution call `agent.start {name:<alias>,kind:"codex",pane_id:<pane>,args:["-c","check_for_update_on_startup=false","--remote","unix://<listenAgent>","resume",T],timeout_ms:30000}`; (5) require that exact TUI connection's successful thread/resume response to return T. Short socket names and both-path byte limits are mandatory. Lost thread/start result is uncertain, not permission to create another thread.

Within the original entered Thread boundary, checkpoint the acknowledged
`thread.id` T durably while Thread remains Dispatching. This consumes neither
the ticket nor the original subscription and grants no Relay admission.
Only when the returned native `historyMode` is `paginated` and `turns` is
explicitly empty, perform the user-authorized native title checkpoint:
exactly one `thread/name/set {threadId:T,name:<SessionCreateParams.name>}`.
Reuse the caller's already-requested display Label, not Herdr's private
action-derived correlation alias; do not invent a title or alter an existing/failed T.
The [native title handler](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/app-server/src/request_processors/thread_processor.rs#L666-L687) must return the schema's correlated empty object.
Only that acknowledgement admits the already-authorized single public
`thread/read {threadId:T,includeTurns:true}` on this same Link/subscription.
Immediately before **each** write, recheck current paired authorization,
the original live ticket/FD, owner/socket epoch and original Herdr source/root
proof. Confirm Thread only after the correlated read returns that exact T,
paginated mode and zero turns, with those identities still current.
Legacy/unknown mode, malformed identity/turns/title acknowledgement,
rejection or lost proof fails closed: no subsequent read, Relay/TUI, retry,
alternate thread, model turn, default/security/owner override or new stage.
Generic timeout, shutdown and revoke settlement retain acknowledged T,
including cancellation during checkpoint admission, and invalidate the
original stream before fence completion. A retained T or a title-set
acknowledgement is not replay permission. Stock deprecation notices remain
enabled; the read's deprecated status is not hidden or treated as fallback.


Correlate exact typed TUI request IDs, capped at64 (including other outstanding calls for collision detection), but change association only on successful thread/start/resume/fork/unsubscribe. First ordinary result/error consumes the observation even if its result is malformed. Unsolicited thread/started never binds or selects. Incoming kernel peer, signed native executable, exact `--remote unix://<listenAgent> resume T` argv, current Herdr terminal/reporter and same owner/socket objects must all join. Mobile remains on launch T; desktop subagent/tab selection is unknown. Every mutation rechecks the actual Target/source epoch, current routing, live presence and loaded membership; no cold fallback.

**Source contract; new-owned Ready verification is pending.** After native
TUI submission, `Topology::source` returning `NotReady` alone
means a transient unavailable read-only readiness snapshot, not a failed
launch or permission to replay. The original creation loop uses its existing
100-ms wait and 300-second deadline; every pass rechecks authorization,
stopping and expiry before looking up the source, including while it is absent.
A recovered source must still match the original manifest's incarnation and
kernel/socket evidence and pass the unchanged exact terminal, loaded-thread,
connection, root/focus and created-binding proofs. Another error, identity
change or propagated native-proof failure still fails closed, retaining its
finite first-gate/ErrorCode uncertainty proof and known resources.
No new source is adopted, no native action is repeated and no deadline is reset.


The user explicitly authorized the stock CLI startup-check override
`-c check_for_update_on_startup=false` only for these Goatr-created TUI
invocations. The [documented boolean](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/config/src/config_toml.rs#L524-L528)
uses the existing [one-off CLI config layer](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/utils/cli/src/config_override.rs);
it prevents the updater menu before the remote connection without installing
anything or saving a config/version dismissal. Owner app-server argv, existing
TUIs, other providers, publisher checks and every other default remain unchanged.
Herdr's submission proof compares all six supplied arguments and the kind token; relay
identity still requires the unique exact remote/resume/thread core among
preserved wrapper prefixes, without imposing this new UI setting on existing
bindings. This is a constant creation behavior, not a new wire/config/profile
knob or automatic retry of earlier uncertain work.

## Restart and restore limits

On companion restart, validate surviving relay/owner epochs and loaded-thread membership before rejoining; no new engine or TUI is started. Server loss or an unloaded thread becomes offline/setup-required, not automatic cold resume. Herdr's launch parser cannot infer identity from `--remote … resume T`, so the relay-owned mapping is authoritative. Do not publish bare `codex resume T` as restore argv or claim automatic shared-mode restore after server/Herdr restart. All helper units terminate only on their own explicit lifecycle or confirmed TUI closure, never companion cleanup.

## Concrete runtime and evidence limits

Ordinary transport owns Codex alongside OMP, routes by the reducer's authoritative provider, and leaves phone open/history as subscriptions/reads of the same installed reducer. Native capture follows all loaded pages, exact resume parameters, owner/presence/loaded before-and-after checks, real response-frame ordering and queued replay with unchanged IDs; there is no replay-completion marker. Turns/items retain native order and opaque continuations, including intra-page continuation when a turn exceeds1,000 items. Completed display bodies belong to the existing State/Content cache, with only bounded source/order/fingerprint metadata retained by the adapter; exact native refetch and COW rehydration never replace live-newer fields. Without an authoritative incremental base, a delta is transient activity rather than guessed appended history. Cache/field/card overflow is explicit unavailable/whole desktop-only, never filtered grants.

`Security::reserve` registers the original only in its successful post-commit callback, before cancellable delivery. Matching receipts cause no job/effect. Newly reserved SessionCreate returns its committed Reserved receipt after original-job registration, without a second reply wait; the original job/native deadline continues. Originals retain paired authorization across disconnect/replacement and15-second response timeout; every subsequent stage still checks revoke and source epochs. Shutdown stops admission, awaits guaranteed StorageOwner admission/no-op, then drains the final owned jobs and native readers without stopping owner/TUI/relay services.

Creation uses workspace → the once-only approved new-root shell proof → owner → thread → independent relay → native TUI → exact Ready. Service-launch process admission is not readiness; actual owner reuse records `ownerStarted:false`. Private v2 `owner-<owner UUID>.owner` preserves action/unit, exact packaged launcher, socket-agent path, original outer argv/cwd/environment digest, manager/service epochs and observed owner/socket identity. The hidden entry accepts only this private record, requires its original manager ancestry, exact unit cgroup, inherited cwd object and unstarted service/epoch fields, and rejects malformed/v1 records without adopting resources. Exact service proof requires `[recorded launcher, codex-owner, --record, exact record path]` while retaining the original configured-wrapper and native-argument checks. Read-only systemd properties use protected `systemctl` and typed `busctl` values, not display-string substring identity. Existing foreign/unproven units, records or sockets are never adopted. Unknown/lost stages preserve resources and do not launch another workspace/thread/TUI or roll back desktop work. `Topology::validate_created_binding` rechecks this action's original shell/root/wrapper proof at Ready without treating its now-running TUI as an idle shell.

Creation selects local `default_codex_owner_id`, or the sole configured owner when omitted; zero owners is setup-required and omission with multiple owners is explicitly ambiguous. Invalid/unknown/duplicate UUIDs fail config validation/preflight. The selector never filters existing attachment/control, adds no wire/UI/profile/environment knob, and does not authorize foreign unit or socket adoption.


The coordinator verified the official0.162.1 Linux-x64-MUSL archive, sole regular native member and its Sigstore bundle against the installed nested ELF **without executing Codex**. The annotated tag object`7a078f6bb4ce0dc2a81408b334ce52979d92c603` resolves to source`092d3acd6bec3e3a14bdc7e7a2810ab628ab759d`; the archive SHA256 is`86f268d81b898f3e144c5ecff2ad2fda2c5802e042d6195b72538fe0fa125057`, its bundle SHA256 is`84a01459f62e7c47f5422c002eb2c22cfea024ab82fc652da392ef946be71e1c`, and the signed member and kernel-opened ELF are both294415048 bytes/SHA`628e35487d888a28f0427fc37501da529dd3d81fcbad7a2c3421c1e8670d92e1`. Cosign3.1.3 returned Verified OK with the existing exact OpenAI release-workflow identity and GitHub Actions issuer, with no insecure overrides. The bounded source/schema comparison retained every adapter-used owner/listen/initialize, loaded/resume/paginated-history, event/item and approval method envelope and root field set; only native removable startup-lock cleanup changed in the inspected Unix listener path. The historic0.159.1/source`8e68a98ef03cdde76d2e6800791ebdf1b3b95b24` record remains provenance for the earlier failed diagnostic, never authority for the current installed binary. The release is mutable; retain object/digest pins, not latest/tag-name authority. This is publisher-linked distribution identity, **not independently reproducible source compilation or live native compatibility**. Runtime hashes a retained kernel-opened executable and rechecks mutable-object/PID/root/socket evidence before/after; its bounded process-scoped proof cache never carries a digest result across owner replacement. Other platform/distribution/proof failures disable only affected capabilities, without name/version/init-only fallback or runtime downloads.

Stock zero-turn feasibility remains a real runtime gate: the pinned `thread_resume_rejects_unmaterialized_thread` test expects “no rollout found” for an unmaterialized rollout. The user explicitly authorized the once-only original-thread native title checkpoint and materializing read described above, after default `thread/start` returns actual paginated mode, before Relay/TUI creation. The original subscription stays open and the exact TUI must still prove successful resume of that same T; source code and local regressions do not establish native resume or desktop readiness. Apart from that title checkpoint/read and the per-created-TUI startup-check suppression above, native defaults remain unchanged. No model warm-up, persistExtendedHistory, alternate thread, provider patch or old uncertain-action replay is used.

A null `thread.path` is valid and is not a missing-session or readiness guard.
The pinned paginated reader constructs that field before awaiting persistence;
the TUI accepts an optional path and validates the returned thread ID. Do not
require a non-null path to explain a TUI Close or change creation admission.
See the [native reader](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/app-server/src/request_processors/thread_processor.rs#L2690)
and [TUI lookup](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/tui/src/lib.rs#L719).

The coordinator ran focused native lifecycle/readiness/security regressions,
Clippy, helper compilation and rooted package builds before the scoped live run.
That real-TTY run reached independent signed owner/TUI/same-thread Ready, an
original successful phone prompt, and successful same-Target Goatr `session.open`
history reads satisfying the mobile-marker-plus-native-tool-record gate. It then
reached the genuine desktop-action barrier and was left live without fabricated
input or acknowledgement. The gate did not separately retain an assistant-role
marker field; no stronger transcript-field claim is made.

The temporary original-invalidation capture was removed after the repaired first owned Codex `!printf` completed on its original Target and generation. The pinned `0.162.1` command item schema permits `aggregatedOutput: null` until textual `item/commandExecution/outputDelta` arrives; the reducer initializes only that null command-output slot before appending the textual delta, while other non-string slots fail closed. The scoped proof stopped before paging or Android acceptance.
Minimal finite presence/creation uncertainty proofs and all permanent native
guards/regressions remain. Cleanup-source checks, a clean rooted package and
final combined-provider Android/desktop/dialog/recovery acceptance are still
owned by the coordinator; the scoped phone checkpoint does not complete them.
Workers did not run source verification checks. Coordinator commands remain the
locked Codex/relay/security integrations and module-local regressions, followed
by full Rust/Clippy/package and actual owned stock-provider/service/Android gates.
The module-local creation response regression covers actual mode/exact-ID/zero-turn admission. The security regression `interrupted_created_thread_checkpoint_retains_id_without_read_replay_or_live_fd` covers Dispatching checkpoint visibility, once-only admission, interruption retention and actual Unix-FD invalidation; these are source additions, not worker-executed or stock-native passes.
The module-local `malformed_title_acknowledgement_cannot_admit_read_or_be_repaired_by_late_reply` regression covers strict empty native name-set acknowledgement and once-consumed typed callback identity; malformed or late results cannot admit the materializing read. This new check is unexecuted by the worker.



## Acceptance gates

See [live session and creation](../ACCEPTANCE.md#live-session-and-creation), [provider dialogs and cancellation](../ACCEPTANCE.md#provider-dialogs-and-cancellation), [generation replacement](../ACCEPTANCE.md#reconnect-and-generation-replacement), [mutation receipts and restart](../ACCEPTANCE.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.
