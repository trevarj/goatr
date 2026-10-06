# Native UI design

Scope: Compose navigation, capability-driven session/conversation rendering, native provider forms, drafts and the Android state store.

[Main plan](GOATR_PLAN.md) · [Tech stack](TECH_STACK.md) · [Pairing/security](PAIRING_SECURITY.md#qr-scanning-and-paste-lifecycle) · [Protocol/state](PROTOCOL_STATE.md) · [OMP dialogs/history](OMP_BACKEND.md) · [Codex dialogs/history](CODEX_BACKEND.md) · [Background delivery](BACKGROUND_DELIVERY.md)

## Navigation and session selection

Implement Compose screens/ViewModels under `app/src/main/kotlin/org/example/goatr/` (the illustrative namespace in [Tech stack](TECH_STACK.md#repository-and-pinned-build-inputs)): Connections/Pairing → Sessions → Conversation; Settings manages delivery and **this phone's host pairings/self-unpair**, not other devices. Link to desktop `goatr devices`/`goatr revoke` instructions for arbitrary-device administration. No terminal WebView/ANSI/key grid/focus synchronization. Connection states Connecting, Synchronizing, Connected-direct, Connected-relay and Offline derive from endpoint lifecycle and observed selected iroh path, not reachability guesses.

Pair confirmation and host Settings state: trusted pairing permits **all supported same-user host sessions**, including outside configured projects. The project picker restricts new creation only. Session rows consume protocol SessionSummary enums/capabilities verbatim, grouped by host/named Herdr server/workspace; do not invent a second status model. UnboundRow visibly explains setup-required/ambiguous/unavailable without a fake chat target. Empty inventory, no projects, incompatible backend, offline cache and partially created resources each have explicit state/action text.

New session selects existing server/configured project/provider/name, validates protocol limits and waits on its receipt. At120-second pending response show ongoing creation and read-only receipt refresh, not another Create submission. Failed/uncertain progress shows known workspace/thread/binding resources and manual desktop reconciliation instructions, never delete/retry automatically. Bottom-sheet switcher preserves this phone's independent selection; desktop tab changes never silently select another session.

## Conversation rendering and provider dialogs

Conversation uses LazyColumn, native user text, selectable assistant Markdown in Markwon TextView/AndroidView and collapsible structured tool cards. Copy always uses original backing text/code whitespace, never rendered Markdown extraction. Disable raw HTML/remote image loading; only explicit tapped HTTP(S) links launch external navigation, reject executable/custom schemes and misleading malformed URLs. ContentRef loads on explicit expansion; expiry refreshes that item or shows unavailable, never an old-generation handle. Show cache/source truncation and unsupported entries, not empty successful content.

OMP history always displays **All branches; desktop-selected branch unavailable**. Use protocol order and parent IDs to show branch/parent cues, not a guessed current leaf; rich persisted text cannot regress to live placeholders. Codex history keeps native turn/item order with live-newer merge precedence.

Pinned pending requests use the finite protocol presentation/answer union. Display all supplied titles/options/details and exact grant scope; large details may expand but cannot disappear behind an actionable summary that conceals permission scope. Multiple cards form one bounded-height scrollable section with a count/selector and composer still reachable; IME insets keep focused field/actions visible. Rotation preserves ordinary form state; secret fields are memory-only, masked, never SavedStateHandle/SQLite/drafts/logs/notifications. Clear secret memory on submit/cancel/card closure, navigation away or process death; avoid immutable long-lived copies.

Use48dp targets, TalkBack labels/traversal heading→question→options→scope→action, scalable text without clipping, validation announcements and explicit disabled reasons. Test narrow portrait/landscape, large fonts, IME, long sequential OMP forms, multi-field MCP and competing pending cards. A cancelled scanner/paste route or browser visit cannot submit an answer.

## Drafts, receipts and local state

Use SQLiteOpenHelper `SessionStore.kt`, one writer exposing StateFlow. Pair metadata, cached completed history, notification events/read-watermarks and receipt bookkeeping follow the **single** [protocol retention/privacy policy](PROTOCOL_STATE.md#bounds-content-and-privacy-retention). No per-token database writes or replicated provider database. Offline cached history is required and labelled stale/partial; a fresh subscription reconciles history/request/receipt state before enabling writes.

Ordinary unsent composer drafts are keyed to host/session/generation and survive rotation in memory. Before sending, atomically persist local action metadata plus ordinary recovery text; the local pre-send receipt is not authoritative delivery and must not destroy the draft. On timeout/disconnect/process death retain bounded recovery, query action.status after hello, display Sent/Failed/Outcome unknown honestly. Only after authoritative dispatched/succeeded receipt is recorded and shown may the recovery copy clear. Unknown/expired/uncertain means inspection/manual explicit new action, never automatic resend or retarget across generations. Secret answers and original pending JSON are excluded even though receipt metadata is persisted before sending.

Disconnected/synchronizing/stale targets disable writes. Forms resolve only from authoritative events/resnapshot, not notification body, local optimistic click or pending-response closure. `notifications.read` advances only when current session is actually displayed. Local Forget is always available offline: delete that host's keys/cache/drafts/receipts/push registration/read/dedup/notifications; explain server authority remains until online self-unpair or desktop revoke. Backgrounding/unsubscribe/forget never kills desktop sessions.

## Acceptance gates

See [provider dialogs](TESTING.md#provider-dialogs-and-cancellation), [generation replacement](TESTING.md#reconnect-and-generation-replacement), [Compose behavior](TESTING.md#compose-behavior), [emulator and camera evidence](TESTING.md#emulator-and-camera-evidence), [notification flow](TESTING.md#background-delivery). These are required implementation checks, not recorded passing results.
