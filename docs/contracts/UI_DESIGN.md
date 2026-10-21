# Native UI design

Scope: Compose navigation, capability-driven session/conversation rendering, native provider forms, drafts and the Android state store.

[Documentation](../README.md) · [Tech stack](TECH_STACK.md) · [Pairing/security](PAIRING_SECURITY.md#qr-scanning-and-paste-lifecycle) · [Protocol/state](PROTOCOL_STATE.md) · [OMP dialogs/history](OMP_BACKEND.md) · [Codex dialogs/history](CODEX_BACKEND.md) · [Background delivery](BACKGROUND_DELIVERY.md)

## Approved MVP presentation

The [MVP plan](../UI_POLISH_PLAN.md) supersedes historical snapshot-only
presentation milestones below. Deliver persistent Workspace/Recent agents views,
three-line expandable cards, chat-style IME-pinned composition, one expandable
activity summary and truthful stable essential state. Live terminals preserve
the native grid with pan/zoom; a single scrollable key row supports one-shot
modifiers and explicit double-tap local locks. Clear modifiers on background,
context or permission loss. These UI requirements confer no native authority.
Missing native/OS support requires its own reviewed authenticated contract.

## Navigation and session selection

The approved presentation direction is a complete Compose interface,
with Sessions / Hosts / Settings as the primary destinations and Sessions as the
landing console. Pairing, creation, conversation, native forms and receipt
details remain first-class flows. Existing engine and native authority stay in
place; see the [rewrite plan](../UI_POLISH_PLAN.md). A host filter is presentation
state and never automatically selects a Target.

Local session-list preferences may show or hide secondary host/workspace context
and enable status headers. Defaults preserve the current complete presentation.
They persist only presentation booleans beside appearance, never identities,
readiness, drafts or receipts. Title, provider and authoritative status remain
visible; exact host/workspace and original Target remain available in row
Details. Headers and visibility never merge rows, select a session or retarget
Open. Unbound rows retain their existing non-openable state.

Settings lists paired desktops through the existing secure host controls.
Add desktop enters normal pairing; selecting a desktop enters its existing
trust/settings screen. Reconnect, local Forget and self-unpair retain their
existing predicates and confirmations. An editable endpoint field must not
replace pairing identity or pretend to provide generic saved-route management.

Implement Compose screens/ViewModels under `app/src/main/kotlin/io/github/trevarj/goatr/` (the publisher namespace in [Tech stack](TECH_STACK.md#repository-and-pinned-build-inputs)): Sessions is the landing console; Hosts/Pairing support connection and trust, and an explicit session row opens Conversation. Settings manages delivery and **this phone's host pairings/self-unpair**, not other devices. Link to desktop `goatr devices`/`goatr revoke` instructions for arbitrary-device administration. The current structured-conversation checkpoint has no terminal WebView/ANSI/key grid/focus synchronization. The active full-parity goal also requires a separately designed authenticated Herdr terminal/shell capability; it remains unimplemented and cannot be replaced with raw provider input or another engine. Connection states Connecting, Synchronizing, Connected-direct, Connected-relay and Offline derive from endpoint lifecycle and observed selected iroh path, not reachability guesses.

Connection presentation separates authenticated live transport from inventory
freshness. Routine refresh never replaces the transport badge with a differently
colored synchronization badge. A reserved refresh-hint area may fade gently;
offline and stale-data warnings remain immediate. Presentation animation never
delays readiness, source, permission, receipt or mutation gates.

Pair confirmation and host Settings state: trusted pairing permits **all supported same-user host sessions**, including outside configured projects. The project picker restricts new creation only. Session rows consume protocol SessionSummary enums/capabilities verbatim, grouped by host/named Herdr server/workspace; do not invent a second status model. UnboundRow visibly explains setup-required/ambiguous/unavailable without a fake chat target. Empty inventory, no projects, incompatible backend, offline cache and partially created resources each have explicit state/action text.

New session selects existing server/configured project/provider/name, validates protocol limits and waits on its receipt. At120-second pending response show ongoing creation and read-only receipt refresh, not another Create submission. Failed/uncertain progress shows known workspace/thread/binding resources and manual desktop reconciliation instructions, never delete/retry automatically. Bottom-sheet switcher preserves this phone's independent selection; desktop tab changes never silently select another session.

First creation does not require an already-bound supported session. With an
authenticated/current inventory, online selected server, available approved
project, valid name and no submitted action, `server.canCreate[provider]`
authorizes an explicit **attempt**. The companion already checks backend trust,
scoped consent, actual evidenced shell/reporting namespace, no-focus context,
project/wrapper/runtime object views and provider owner/relay prerequisites;
every later stage still needs the new root's matched once-only proof and native
binding. Backend live readiness/reason remains visible and truthful, including
`setup_required` when no live binding exists; the UI must not require `Ready`
to bootstrap one or invent a binding to make the button available. Stale,
offline or false-canCreate inventory remains disabled. Native Ready is an
observed final creation receipt, never a display eligibility claim.


Under-the-hood inventory uses `HostSnapshotResult.hierarchy` and the native
workspace/tab/pane arrays, including shell-only, unsupported and returned empty
workspaces. Use returned labels, parent IDs, numbers/order and focus directly;
Goatr server/session/cache aliases are not native desktop entities. Pane
Target/unboundRowId is the only explicit supported association in a current
incarnation. Native idle/working/blocked/done/unknown decorates the pane; provider
status, capabilities and conversation title retain their separate meanings.
No geometry/terminal facade, shell controls or automatic desktop-focus selection
is added. Missing/unsafe/too-large display and coarse restore_failed reasons stay
visible; unavailable/too_large empty arrays do not mean an empty desktop.

Frozen phone-owner APIs are `ConnectionView.inventory`,
`ConnectionView.inventoryReady[host]` (direct projection of existing
`HostReducer.ready` plus authenticated live transport), and each
`NativeHierarchy.freshness/reason`. Retained inventory is stale/synchronizing
when inventoryReady is absent/false, regardless of its last native-current
fields or selectedPath. An authoritative host snapshot can still contain a
stale/unavailable native source. Focus never calls session.open or changes the
independent phone Target. Closure/replacement shows the selected row offline or
changed instead of automatically selecting another pane.

`ConnectionView.mutationReady[host]` separately projects authenticated, live,
current host inventory and completed host receipt reconciliation. Mutation
controls require this gate plus their existing Target, source, reducer and
capability checks. Readable inventory/source identity alone does not enable
Send during receipt recovery. Publish the disabled gate before the first
suspension after clearing receipt readiness; teardown publishes its disabled
transport state before awaiting cleanup. Final admission still rechecks authority.
Readiness can change after a local action commits; retain that action and its
known write state for receipt reconciliation, with no automatic resubmission.
A current invocation whose initial local-action lookup is absent may retain an
ephemeral wait ticket before its single pre-send commit. The actual pre-send
decision remains authoritative: existing actions stay status-only. The ticket
binds the original Live, immutable request/body/action, boot, Target and caller.
Only already-active recovery on that same Live may be awaited before the commit
or while its first frame remains unstarted. One absolute 15-second admission and
request budget (120 seconds for creation) starts before this precommit wait and
includes key preparation, the single commit and subsequent waiting. It never
refreshes. Recovery waiting happens
outside writer/operations locks and uses the existing recovery's reads. Every
suspension is followed by original identity/readiness checks; canonical committed
receipt reconciliation returns that receipt without another frame. Existing
local actions remain status-only. The writer revalidates before the first frame;
once frame writing starts, only the existing five-second frame lifetime applies.
Receipt persistence/publication stays after the request timeout boundary.

Existing `HostReducer.sourceCurrent(serverId)` derives the mutation gate from
that same inventory/readiness: known offline/stale native sources disable writes.
SourceUnavailable always disables this gate, including an online server with
unavailable empty arrays after display overflow. Display-only too_large is
reserved for a confirmed current source and does not disable otherwise-proven
supported sessions or depend on creation-shell capability. Native dirty
host.changed invalidates readiness even for shell-only overflow inventories;
reducing the graph never makes its source current.

`SessionStore.cacheInventory` stores one bounded safe last display per paired
host in the existing SQLite/privacy/Forget owner. `StoreView.inventory` exposes
`CachedInventory(hostId,inventory,savedAt)` with stale always true; native graph
associations are stripped and current graphs become stale before persistence.
It never installs a HostReducer snapshot or grants mutation/readiness authority.
Use current/retained connection inventory and cached stale display distinctly,
not a cache/live merge by native route strings. Schema version2 adds this cache
table to existing version1 private databases without changing keys or receipts.
Actual Compose presentation and live desktop parity remain separate acceptance
work; this change adds no UI scaffold.

The shared `ProtocolFixtureTest.hierarchyOverflowPreservesSourceUnavailability`
check covers both unavailable-source rejection and current-source display
overflow/control independence. JVM and installed after-repair execution require coordinator validation.


Compose owns Light/Dark/System colors. The app-local Theme.Goatr disables
automatic Force Dark on API29+ so the light platform base does not recolor
Compose controls; older APIs retain the original base. Device settings and
authorization are unchanged.

Use official Material icons for navigation and recognizable app-owned actions,
including creation, reconnect/refresh, Send/Stop, Copy and disclosure controls.
Retain visible labels where helpful, existing semantics and touch targets. Icons
never replace native permission text, choices or scope, and native label text
does not determine an invented permission meaning.

## Control panels and primary actions

Agent/session control surfaces place a compact Material icon control bar above
the chat input and use FABs for primary actions. Supported controls retain their
existing exact Target, readiness, permission, reservation and receipt predicates.
A connected transport does not imply mutation readiness. Composer and creation
notices describe the actual existing gate: foreground/transport, current
operation, inventory, receipt reconciliation or native source/session readiness.
Presentation checks dependent gates after their prerequisites, so a withheld
source cannot mask an already-stale inventory. This adds no authority or new
readiness reducer. Reserve notice space to reduce layout shifts.
Stop remains explicit and separate from Send. Layout must reserve reachable
controls with the IME open and avoid transcript/form overlap. Icons have
descriptive labels and 48dp targets; explanations remain accessible. The same
pattern applies when authenticated terminal capabilities are implemented; it
does not grant raw shell/provider input authority.

## Planned ordinary-shell workflow

Expose Terminals from the existing host/workspace hierarchy. List positive shell
summaries separately from provider sessions; cached rows are offline and cannot
act. Open reads an exact ShellTarget into a secure memory-only viewer with
visible Refresh and, when granted, a New terminal here FAB. Dismiss the IME;
show no composer, keys or close-shell placeholder in this slice.

Capture original Live/host/subscription/source/Target and selection ticket across
reads; cancel and clear text on background, disconnect, scope/generation loss or
navigation. Creation confirmation names the original host/workspace/terminal.
Reuse existing action storage, receipt inspection and separate-creation consent
latches. Pending/uncertain actions cannot automatically resend or clear their
latch; receipt refresh updates the gate without granting another action. Native
IDs alone do not make a created terminal ready. See
[wire/receipt contract](PROTOCOL_STATE.md#ordinary-shell-consumers-planned).

## Planned shell Text/Keys workflow

Enable controls only from the original positive input capability. Keep a secure
IME-pinned editor/Send and Text/Keys mode, with output scrolling independently.
Text sends exactly the captured draft; Enter is a separate key. Provide named
navigation/editing/F1–F12 keys and one-shot modifiers, one fresh action per gesture.
No hold-to-repeat, automatic batches or implicit draft sending.

Keep draft/cursor/mode in ViewModel memory by exact host/ShellTarget, at most16
entries and16KiB per draft; never silently discard unsent text or transfer across
generations. Clear modifiers on target change, background and after use. Reuse
the original opaque viewer lease through capture and frame admission; after
admission preserve the action owner despite navigation. Keep one unresolved
input per target. Show Input accepted rather than command completion; next
explicit gesture follows a displayed authoritative success/failure. Unknown or
Uncertain requires separate-action consent joined to the original receipt UUID.
Retain current status observations separately from historical receipts. Missing or
evicted observations never imply Known; use the existing explicit Check receipt
action to regain current evidence, and pin the exact observation through frame
admission. A secure selected-host Clear local drafts confirmation works offline
and for stale targets, clearing only text/cursor/mode and sent-text tracking;
preserve unresolved latches, receipts and admitted jobs.
Clear sent text only after displayed acceptance and an unchanged draft revision.
No input is persisted in SQLite, SavedState, logs or notifications. See the
[wire/receipt boundary](PROTOCOL_STATE.md#planned-textkeys-input).

## Secure terminal snapshot

Add a labelled Material Terminal action to the conversation control menu only
for the selected live reducer, current source and true `terminalRead` capability.
Open a secure dialog with nonselectable plain monospace text, explicit Refresh
and Close, loading/error/empty states and a visible truncation notice. Do not
render Markdown, activate links, offer Copy/share or accept terminal input.

The request captures the original connection instance, Target, reducer and
source; validate them inside the existing request writer lock and on response
admission. A canceled dialog ticket rejects delayed pending callbacks. Close,
Refresh, selection/navigation change, source loss, disconnect and backgrounding
cancel the request and clear text. Persist no output or dialog recovery state.
A changed session never silently refreshes or retargets the old dialog.

## Secure Worktrees list

Planned flow: a labelled Material Worktrees action requires the selected live
reducer, original current source and true `worktreesRead`. Show a secure,
memory-only list with path/label, native branch/head and flags, explicit Refresh
and Close, and loading/error/empty states. Unknown linked-worktree status remains
unknown. Use existing selected-session context; infer no repository metadata.
Paths are nonselectable plain text, with no links, Copy, Open or mutation action.

Use a separate cancellable request ticket. Recheck the original connection,
Target, reducer and source inside the request writer lock and on response
admission. Clear results and cancel on Close, Refresh, navigation/selection,
source loss, disconnect or backgrounding. Never restore, persist or automatically
refresh a replaced dialog. This list does not complete Changes, worktree creation,
opening/removal, file operations or terminal-control parity.

## Secure Changes view

A labelled Material Changes action requires its own
current original context and `changesRead`. Use a secure, memory-only dialog
with status rows and staged/unstaged patch sections, explicit Refresh/Close,
and honest binary/untracked states. Render nonselectable plain text with wrapping,
no ANSI, links, Copy/share, staging or commits. Explicit file, line and hunk draft
references are the sole content-insertion exception. A secure confirmation previews
the exact ordinary text before appending it to the same current conversation draft.
File references use `File reference: <host-relative path>`. Line/hunk references
identify the observed staged or unstaged patch and its old/new positions; staged or
removed positions must not masquerade as current working-tree lines. Derive selection
and content only from the original bounded Changes snapshot and exact entry membership,
never arbitrary UI text, extra file reads or native calls. Unsupported, ambiguous,
quoted or binary diffs remain visible as raw patches with an honest reference limitation;
file references remain available. Require the original snapshot object/ticket, Target,
source, paired connection, Ready `changesRead` reducer and foreground admission at
confirmation. Revalidate and append without suspension. Reject an oversized combined
PromptText whole, preserving the draft. Stale selections fail visibly; clear pending
selection/errors when the dialog closes or context is lost. Keep native previews under
the secure window and sensitive `changes_body` ancestor. This never uploads, opens,
shares or sends a file, copies to the clipboard, persists a new value, or submits a
prompt. Reuse Worktrees writer/reply
fences and cancellation on navigation, source loss, disconnect and backgrounding, with a
separate ticket. No persistence or automatic refresh.

## Rename workspace

Expose a Material icon action for the explicitly selected original Target only
when workspaceRename is supported and existing mutation gates permit it. A
scoped dialog shows the current workspace label, the existing Label bounds,
Save and Cancel. Workspace label remains separate from conversation title.
Keep edits in memory and clear them on host/source/Target loss. Show the actual
pending, succeeded, failed or uncertain receipt; inspect uncertain outcomes
without another submission. Refresh authenticated hierarchy for the resulting
label, without optimistic rename or label persistence in receipt metadata.

## Conversation rendering and provider dialogs

Conversation uses LazyColumn, native user text, selectable assistant Markdown in Markwon TextView/AndroidView and collapsible structured tool cards. Copy always uses original backing text/code whitespace, never rendered Markdown extraction. Disable raw HTML/remote image loading; only explicit tapped HTTP(S) links launch external navigation, reject executable/custom schemes and misleading malformed URLs. ContentRef loads on explicit expansion; expiry refreshes that item or shows unavailable, never an old-generation handle. Show cache/source truncation and unsupported entries, not empty successful content.

OMP history always displays **All branches; desktop-selected branch unavailable**. Use protocol order and parent IDs to show branch/parent cues, not a guessed current leaf; rich persisted text cannot regress to live placeholders. Codex history keeps native turn/item order with live-newer merge precedence.
When an authenticated session exposes an opaque older cursor, the visible `Load older native history` control uses that same selected Target. The owned stock UI test obtains an independently proven older ID from the already-generated source, taps the real control, and waits for that ID in the displayed reducer history; it does not copy fixture history or open another provider session.
An accepted source-bound older page moves the single bounded local display window
toward older records. At its unchanged1000-row/byte limits, older browsing evicts
newer rows, including on subsequent live upserts, while retaining the server's
next cursor so the same native view can continue. This local window does not claim
unlimited provider retention. Live state, pending requests and newer field
precedence remain current.
The visible `Return to latest history` control uses the existing fresh snapshot
operation and resets this window; a stale page cannot change its mode. The stock
test follows at most32 actual older-page controls on one Target/subscription,
asserts the independently proved row actually displayed, then returns to latest.

Pinned pending requests use the finite protocol presentation/answer union. Display all supplied titles/options/details and exact grant scope; large details may expand but cannot disappear behind an actionable summary that conceals permission scope. Multiple cards form one bounded-height scrollable section with a count/selector and composer still reachable; IME insets keep focused field/actions visible. Rotation preserves ordinary form state; secret fields are memory-only, masked, never SavedStateHandle/SQLite/drafts/logs/notifications. Clear secret memory on submit/cancel/card closure, navigation away or process death; avoid immutable long-lived copies.

Use48dp targets, TalkBack labels/traversal heading→question→options→scope→action, scalable text without clipping, validation announcements and explicit disabled reasons. Test narrow portrait/landscape, large fonts, IME, long sequential OMP forms, multi-field MCP and competing pending cards. A cancelled scanner/paste route or browser visit cannot submit an answer.

## Saved Iroh reachability (planned)

For an already authenticated paired host (stored deviceId present), allow up to
eight direct literal IPv4:port or [IPv6]:port hints, port1–65535, each≤128 chars,
and one HTTPS relay URL≤2048 chars. Reject hostnames in direct hints, IPv6 zones,
duplicates, relay credentials/query/fragment/non-root paths. Validate and
canonicalize locally without DNS or a connection attempt. Adding a desktop
still requires normal pairing; hints never change its identity or recreate keys.

Persist validated hints and a monotonically increasing revision in a separate
host_routes table, using SessionStore's writer/transactions/StateFlow and host
Forget cascade. Existing installations migrate to empty hints; concurrent Hello
metadata updates cannot overwrite them. Save does not restart a healthy or
in-flight connection. Offer normal explicit Reconnect separately.

Snapshot hints/revision into the existing ConnectionAttempt and pass all hints
to one EndpointAddr with the original pinned endpoint ID. Preserve N0 discovery,
one retained endpoint, ALPN/remoteId/Hello/device checks, deadlines and exact
receipt/Target recovery. No per-hint retry engine or automatic action replay.
Show “Hints supplied for this attempt”; never claim each was attempted. Saved
revision changes show pending configuration for the next attempt.

Use actual PathSnapshot id/remoteAddr/isIp/isRelay from paths/watchPaths; refresh
after Lagged. Publish only for the authenticated, alive, installed Live identity
and clear on teardown. A canonical match may say “Current path matches this saved
hint”, without claiming the hint caused discovery. Otherwise show Other Iroh
path or Current path unavailable. Removing a hint affects future supplied hints;
it neither excludes that address nor purges Iroh discovery/cache state. The
existing ABI supports multiple direct addresses and exactly one relay URL.

## Provider Skills catalogue (planned)

Codex may expose optional-false skillsRead and read-only skills.list for the
original Target. Pin the existing correlated thread/resume response's top-level
cwd to its original connection, native thread, owner and source epoch after
existing admission checks. Require an absolute UTF-8 path≤4096 bytes without
controls. Missing/invalid cwd disables Skills alone; never infer it from a
configured project, persisted thread metadata or current terminal cwd.

Arm settings observation before resume and inspect thread/settings/updated at
native frame ingress before queued callbacks. Changed/malformed relevant cwd
invalidates this connection's Skills context, without replacing its pin.
Same-cwd settings invalidate previous catalogue results. Use one existing-owner
skills/list request with cwds:[accepted cwd], forceReload:false; recheck original
connection/Target/source and catalogue generation after awaits and before
publication. No extra resume/read, shell discovery or shared-cache clearing.

Return at most128 skills,32 native load errors and256KiB aggregate projection,
also enforcing the actual encoded frame cap. Reject overflow rather than imply
a complete truncated catalogue. Include bounded names/descriptions, optional
display metadata, scope, enabled state and exact native paths. Label the view
Available skills for this thread directory; this is not loaded-thread state.
Dependency commands, remote icons and default prompts remain inert.

An in-memory searchable picker captures host/Target/source/list ticket/catalogue
generation and rechecks foreground, selection and capability before display or
insertion. Insert only an exact-path mention that round-trips through the pinned
native grammar; reject unsupported names, excluded environment-variable names,
closing parentheses in paths and trimming changes. No guessed bare-name or
encoded/escaped fallback. Preserve draft/focus; selection never submits or
creates a receipt. Clear catalogue authority on settings/source loss, disconnect
or Forget. Disabled/unrepresentable entries remain inspectable.

OMP exposes enabled commands from its original public getCommands catalogue when the independent USER-preview grant and authenticated original-process channel are present. The secure picker labels command-text insertion honestly: it does not expand, invoke or send the skill. View content displays only a complete size/SHA-256/strict UTF-8-verified document up to1MiB, transferred in64KiB chunks. Source, foreground, selection, catalogue or ticket loss clears the memory-only document. Unrepresentable command names remain inspectable without insertion. Codex full-document preview and native OMP skill-invocation acknowledgement remain unavailable. Skills is distinct from local snippets.

## Local composer snippets

The planned Snippets picker stores only explicitly user-authored entries on this
phone, scoped to a paired host. Create/edit/delete are local operations; saving
discloses persistence and never imports a conversation, recovery draft, native
form or secret answer automatically. Provider Skills remain separate from snippets and follow the
[original-process OMP catalogue and complete-preview contract](OMP_BACKEND.md#enabled-skills-extension-channel).
Codex full-document native authority and OMP invocation remain outstanding. Each entry has a random local UUID, title up to80 Unicode
code points/320 UTF-8 bytes, and nonempty body up to8192 UTF-8 bytes. Limit32
entries per host and128 total; reject capacity without silent eviction.

Insert captures the original ConversationKey when the picker opens and rejects
a changed selection/generation or missing pairing. Append to that key's ordinary
draft through the existing composer/reservation path, respecting PromptText
bounds; never overwrite another draft, send, invoke a provider, or reuse a
receipt/action. Dismissal and navigation perform no insertion. The existing
Send readiness, permissions, recovery and receipt path remains authoritative.

Use the existing serialized SessionStore/StateFlow and additive private schema
migration. Saved snippets persist until explicit deletion or host Forget, whose
foreign-key cascade removes them. Unsaved editor text is memory-only; backup
exclusion, secure deletion and no-content logging remain unchanged. See the
[storage bounds](PROTOCOL_STATE.md#bounds-content-and-privacy-retention).

## Drafts, receipts and local state

Use SQLiteOpenHelper `SessionStore.kt`, one writer exposing StateFlow. Pair metadata, cached completed history, notification events/read-watermarks and receipt bookkeeping follow the **single** [protocol retention/privacy policy](PROTOCOL_STATE.md#bounds-content-and-privacy-retention). No per-token database writes or replicated provider database. Offline cached history is required and labelled stale/partial; a fresh subscription reconciles history/request/receipt state before enabling writes.

Ordinary unsent composer drafts are keyed to host/session/generation and survive rotation in memory. Before sending, atomically persist local action metadata plus ordinary recovery text; the local pre-send receipt is not authoritative delivery and must not destroy the draft. On timeout/disconnect/process death retain bounded recovery, query action.status after hello, display Sent/Failed/Outcome unknown honestly. Only after authoritative dispatched/succeeded receipt is recorded and shown may the recovery copy clear. Unknown/expired/uncertain means inspection/manual explicit new action, never automatic resend or retarget across generations. Secret answers and original pending JSON are excluded even though receipt metadata is persisted before sending.

After the exact same-host `SessionCreate` receipt is authoritative, expanded and visibly shown with a terminal `Succeeded`, `Failed` or `Uncertain` outcome, the user may explicitly start a **separate** creation. The confirmation is joined to that prior action UUID and rereads the current displayed local receipt; it rejects a changed host, a different active creation latch or any other pending creation. `Uncertain` names the prior action, shows its bounded known retained resource identifiers, and warns that it may already have effect while those resources stay untouched. Confirming clears only the matching in-memory form/latch, requires a newly entered name and creates a fresh UUID through the ordinary guarded Create path; it never alters, retries, cleans up or retargets the prior receipt/resources.

Disconnected/synchronizing/stale targets disable writes. Forms resolve only from authoritative events/resnapshot, not notification body, local optimistic click or pending-response closure. `notifications.read` advances only when current session is actually displayed. Local Forget is always available offline: delete that host's keys/cache/drafts/receipts/push registration/read/dedup/notifications; explain server authority remains until online self-unpair or desktop revoke. Backgrounding/unsubscribe/forget never kills desktop sessions.

Foreground source/cache consumers use `ConnectionView.sources[host][Target]`,
the installed authoritative subscription UUID, exposed only while the exact
host/native source/session is current and ready. Existing owner
`content(host,source,params)`, `cachedContent` and completed-only `cacheContent`
recheck that same Target/source before and after work; pending/activity payloads
have no cache admission. Snapshot cache stores that UUID, never a UI lifetime
counter or a relabelled old loaded body. `leave(host,target)` resolves and drains
the actual current subscription after automatic reopen, not the initial open ID.

`ConnectionView.latestReceipt[host]` is one coalesced read-only inspection hint
from the existing serialized owner, read back from canonical committed
`SessionStore` metadata after its authority/transition guard. It is not a second
reducer, pre-send delivery claim, readiness grant or replacement for durable
`localActions`. The facade reads only the selected host's current bounded
100-action UUID-keyset page (explicit next/first controls) plus one inspected
action, on receipt/hello recovery changes or explicit inspection—not a10,000
action/all-host scan per transient activity update. Exact receipt display
precedes `outcomeShown`; failed/uncertain/unknown copies remain inspectable.

Retained ordinary form/composer memory is not SavedState. Clearing secret
answers and pairing input also disposes the corresponding input composition,
dropping framework editing/undo references rather than only blanking visible
text. Foreground stop cancels startup/UI waiters, then awaits the existing
native owner's cleanup; it never stops desktop work. The exported launcher is
single-task, with inset consumption and separately scrollable bounded chrome
so IME/large fonts cannot consume the entire conversation action viewport.

UI waiter cancellation is separate from the existing transport owner's frame
lifetime. Cancellation before writer admission sends nothing and does not close
a healthy connection. An admitted frame finishes under the existing five-second
write bound; genuine partial-write/deadline failure still invalidates transport.
Canceled or response-timeout waiters retain only the existing method/deferred/
apply correlation, capped by the same64 in-flight permits, until a strictly
validated reply or connection close releases the slot. Unknown/duplicate
responses remain fatal; no request is retransmitted or retargeted. Late replies
retain existing authority/source/cache guards. Explicit local expired/stale
consumer rejection fails the waiter rather than invalidating otherwise valid
transport. An abandoned `session.open` is never installed/cached; its exact
returned source is unsubscribed outside the reader under existing operations,
without retiring a newer source. Native cleanup awaits admitted writes.

Pending requests occupy their own bounded scroll region with a visible count
and native request selector, independently of transcript scroll. The ordinary
composer has its own bounded scroll region rather than a lazy transcript tail.
Opaque secret question IDs use disjoint draft/answer storage roles, never a
suffix-encoded shared secret map. Numeric fields request the installed signed
integer/decimal IME modes while retaining the same exact protocol validators.
Native Markdown selection/keyboard/accessibility Copy is labelled and routed
to the original backing text, not Markwon's rendered spans.

Zero-host protected-key loss has an explicit unpaired-installation Forget
confirmation. The existing `key_deletion` table records a distinct LOCAL
installation scope (not an EndpointId or host row), guarded by an empty host
table. Existing owner shutdown and protected alias/ciphertext/marker deletion
complete before acknowledgment; startup can finish only that already-consented
obligation. Key loss alone never creates a deletion intent or replacement key.
This does not change paired-host Forget or server revocation authority.

CameraX asynchronous errors are owned by a fresh binding as well as the existing
route/disposal guard. Shared CameraInfo terminal replay from the prior attempt
does not fail a new binding before its own PENDING_OPEN/OPENING/OPEN state or
first real frame; thereafter every live error, including CLOSING/CLOSED, reaches
explicit paste/Retry fallback. Provider/bind/analyzer failures remain immediate.
Only the owned observer/use cases are removed. Accepted analysis tasks drain with
orderly executor shutdown so their native frame future/finally cleanup runs.
Inverted terminal QR polarity uses the installed QR-only MultiFormatReader
inversion path, not an ignored QRCodeReader hint.

The coordinator recorded the complete isolated API34/x86_64 native camera
six-phase preview/Back/reentry/paste/policy-error/Retry/distinct-fresh-QR/close
pass with original source/pose/policy restoration. Temporary diagnostic bindings,
image decoder, snapshots and privacy overrides are removed. This is actual
CameraX/ZXing component evidence, not live pairing authorization, physical camera,
stock-provider or real16KiB proof. Other source contracts still require their own
matched UI/native/provider evidence.


## Acceptance gates

The approved [Compose interface rewrite](../UI_POLISH_PLAN.md) covers the full
Android presentation. Earlier Phase 0/1 labels remain only in the deferred
acceptance checklist for the later exercised checkpoint. Development APKs
may be handed over for immediate feedback with known failures disclosed; use
incremental builds and checks scoped to changed behavior. The
[checklist](../ACCEPTANCE.md#first-testable-apk-ui-handoff) remains pending work,
not a prerequisite for development feedback. Identity, permission, privacy and
no-replay protections remain required. The owner accepts actual owned-APK
screenshots for visual review and explicitly defers the dedicated accessibility/
TalkBack audit and new release-like comparative performance harness. Preserve
existing semantics and touch targets; report those audits as deferred, never
passed. Screenshots do not replace functional/native checks or prove performance.
Later voice, native command/queue and attachment phases retain their prerequisite
contracts and feature-specific proof. Current text `submit_or_steer` controls
must not advertise unsupported distinct native actions. The explicit exhaustive
paging deferral changes only its named evidence gates, not these UI requirements
or product history semantics. These are requirements, not implementation/pass
claims.

See [provider dialogs](../ACCEPTANCE.md#provider-dialogs-and-cancellation), [generation replacement](../ACCEPTANCE.md#reconnect-and-generation-replacement), [Compose behavior](../ACCEPTANCE.md#compose-behavior), [emulator and camera evidence](../ACCEPTANCE.md#emulator-and-camera-evidence), [notification flow](../ACCEPTANCE.md#background-delivery). Each gate requires its own matched evidence; the isolated recorded camera component pass above does not grant the other gates.

## Planned agent alias control

The selected conversation offers Agent alias only with original foreground,
Ready source and positive agentRename capability. Use a secure ephemeral dialog
with the exact native alias grammar; explain that the value is a desktop agent
alias. One explicit submit produces one action UUID through the current mutation
owner, dismissing its editor and showing the durable receipt. No optimistic title
change, automatic retry, stale-source submission or clear-alias action.

## Planned safe worktree removal

A linked-checkout row with a native candidateId may offer a labelled Material
Delete action only with positive worktreeRemove and current original mutation
authority. Fetch a bounded native preview, then show a secure memory-only
confirmation containing the full native path/branch and the instruction to stop
external tools. Do not close any desktop terminal on the user's behalf. Pin
host, Target, source, ticket and preview through request admission; invalidate
unsubmitted confirmation on background/navigation/source changes. One explicit
confirmation creates one action UUID. Show confirmed/refused/uncertain outcome
and explicit Refresh, with no optimistic deletion, force or automatic retry.

## Planned attachment selection and insertion

Use the Android system Photo Picker/OpenDocument from a labelled Material action.
Capture original host/Target/source/picker ticket before launch. Prepare selected
original bytes in bounded encrypted app-private staging, enforcing actual bytes
rather than reported size; exclude backup, never log URI/content, retain URI
access only where supported and clear on expiry/discard/Forget. Do not claim
transcoding or metadata stripping. Returning from a picker does not authorize
upload: foreground/source admission requires an explicit Upload gesture.

A secure preparation/progress/recovery sheet shows limits/expiry and truthful
cancel/unknown results. Each frame rechecks the same original Live/reducer/Target/
source and user lease through frame CAS. Completed upload offers Insert path,
with original conversation plus draft revision/cursor rechecked synchronously.
A changed draft needs a fresh explicit insertion, never overwrite, retarget or
automatic Send. Lost source retains recovery facts, not action authority.

## Agent process directory terminal creation (planned)

The selected original conversation exposes a Material New terminal in agent process directory action and secure confirmation. Explain that this is the original provider process OS directory, which can differ from its conversation directory; no phone path selector or cached cwd is offered. Unsupported native support/grant/process proof is stated clearly.

Pin original host/Target/source/subscription/foreground confirmation through frame admission. Create exactly once under a fresh UUID, show the retained receipt and require explicit separate-action consent joined to its prior UUID before another creation. Source/navigation/background loss revokes pre-admission only; an admitted action retains its receipt. View terminals is explicit; Open created terminal requires Succeeded plus the exact current positively mapped ShellTarget/identities. Otherwise refresh normal terminal inventory and choose a fresh row. Never focus desktop, send input, infer a directory or adopt uncertain output.

## Secure Files browser and export (planned)

A Material Files action appears for an artifact-capable original conversation or ordinary terminal row, including artifact-only rows whose terminal read grant is off. The secure browser shows memory-only directory breadcrumbs, bounded cards, explicit paging and refresh. It has no write/delete/rename/search/recursive operation or automatic external viewer.

Text preview collects the complete file, verifies exact size/SHA-256, accepts strict UTF-8 up to1MiB and only tab/CR/LF controls, then presents secure monospace text. No partial preview, WebView, clipboard or extension-based trust.

Export requires a separate confirmation naming the file and size and disclosing that the chosen document provider receives its name/content. CreateDocument(application/octet-stream) is the only destination selector, using a sanitized128-byte name. No native file is opened/read before the result. A single shared attachment/export picker slot retains the original RuntimeClaim for at most two minutes. Picker return must preserve the same original Live/reducer/subscription/source/Target and selected ref; only its fresh foreground token is renewed. Loss/expiry rejects the result without writing or deleting an unproved URI and discloses a possibly empty document.

An admitted export opens only that returned URI with rwt, streams bounded64KiB chunks while hashing and requires full size/SHA, flush/close and fsync where supported before completion. Background, cancellation or source loss stops subsequent reads/writes and attempts exact lease/created-URI cleanup. Failed deletion explicitly warns about a partial document. There is no persisted URI grant, process-death resume, background export, automatic send, draft edit or recovery adoption.
