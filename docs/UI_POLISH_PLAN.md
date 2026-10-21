# Goatr mobile MVP

## Goal and completion

Deliver a device-validated Android MVP for using the same original OMP and
Codex conversations on mobile, continuing them on desktop, then returning to
mobile with foreground reconnect and native history intact. Complete every
feature-matrix row below. Herdr remains the sole agent and terminal authority.

Use the paired Mi 9T (ADB serial 74110723), Wi-Fi and background operation.
Mobile data is not required. Preserve app data and pairing. Use actual-device
screenshots for visual checks, excluding pairing and sensitive scenes; verify
native effects separately. Fresh normally created owned sessions may establish
test originals; preserve those identities throughout the continuity workflow.

MVP completion requires targeted regressions, all matrix workflows on the phone,
and independent high UI/code review. Incremental development APKs remain usable
feedback artifacts with disclosed failures. Exhaustive clean builds and release
qualification are a later checkpoint, not prerequisites for development feedback.

## Implementation and review

One coordinator owns integration, device verification and Git. One GPT-6.1 Sol
medium implementation writer works in an isolated worktree; a GPT-6.1 Sol high
reviewer signs off before every commit. No recursive delegation. Preserve dirty
native work. First finish the functional UI component split and active branding
cleanup, add scoped ignores, and make the authorized signed bulk baseline commit.
After that, use signed commits between reviewed feature slices; no pushes.

Checks follow completed implementation/review handoffs and cover changed behavior.
Use at most two build cores/jobs and one heavy build at a time. Keep exact source,
APK, commands, actual results and blockers in [status](STATUS.md). Do not promote
source review, compilation, screenshots or synthetic fixtures to native proof.

## Presentation contract

- Sessions offers persistent Workspace and Recent agents views. Workspace cards
  reveal their agents; agent cards open chat. Collapsed cards contain at most
  three text lines, with explicit expansion. Long press exposes secondary actions
  and technical details without changing the selected native session.
- Chat uses readable Markdown/code, contained user messages and an independently
  scrolling transcript. A stable multiline chat composer stays above the IME,
  with one compact Material icon bar above it and explicit reachable Send/Stop.
- Combine routine tools into one expandable activity summary. Keep questions,
  approvals, failures and uncertain actions visible and actionable independently.
- Show simple, truthful essential state. Hide diagnostic identifiers, epochs,
  transport and receipt internals in details. Reserve status space and use gentle
  transitions; routine refresh must not flicker or move content. Authority and
  permission loss still disable actions immediately.
- Use Material 3, official Material icons, comfortable wrapping and touch targets.
  Compact density must preserve readability and keyboard reachability.
- Terminals are live interactive native-grid views with pan/zoom, no wrapping
  that changes the grid and no silent desktop resize/focus. Provide one scrollable
  key row with Ctrl/Alt/Shift/Super, Esc/Tab, navigation and F1–F12; FAB controls
  expose keyboard and terminal actions. A modifier tap applies to the next key;
  double tap locks it locally with a visible indicator. Reset modifiers on
  background, context or permission loss. No input repeats or implicit sending.

## Feature matrix

Implemented means source exists; verified means the actual workflow passed.
Missing native or OS support remains required work, never a disabled substitute.
Detailed authority stays in the linked topic contracts.

| Workflow | Current boundary | Required phone result |
|---|---|---|
| Console, Hosts, pairing | Installed development APK connects; filters and inbox exist, interaction partly unverified | Persist view choice; compact expandable workspace/agent cards; secure pairing, truthful states and host recovery |
| Chat and native history | Original OMP mobile prompt/reply proved on a prior build; current Codex flow blocked before creation | Both providers: prompts, Markdown/code, tools, native questions/approvals, Stop, genuine older browsing and stable drafts |
| Mobile/desktop continuity | Historical OMP round trip exists; current both-provider round trip incomplete | Same original sessions mobile→desktop→mobile; foreground recovery/history without retargeting or replay |
| Agent/workspace management | Alias/rename and configured-project creation implemented, partly unexercised; same-workspace placement incomplete | Rename, explicit scoped Close, create beside current agent and select authorized directory |
| Live terminals | Bound snapshots/read/create/input implemented; live grid and safe Close incomplete | Live output, shell inventory/create/open/close, Text/Keys, modifiers/navigation/F1–F12, retained drafts and focus |
| Local snippets | Phone create/edit/save/insert/clear/delete passed historically | Comfortable editor; insertion never sends automatically |
| Provider Skills | Catalogue, insertion and OMP preview implemented; invocation and Codex document authority incomplete | Search, complete authorized preview, exact draft insertion and native invocation for both providers |
| Attachments | Picker/encrypted preparation/upload/recovery/insertion implemented, workflow pending | Photo/file selection, progress, cancellation/background recovery and host-path insertion without sending |
| Artifacts | Authenticated inventory, secure preview and export implemented, workflow pending | Browse, bounded original-context preview, select/cancel and SAF export |
| Worktrees | Inventory/create/opaque-candidate removal implemented, positive workflows pending | Target-bound list/create and scoped confirmed/refused/uncertain removal; no unrelated effects |
| Git review | Status/context and file/line/hunk references implemented, workflow pending | Staged/unstaged per-file review and explicit references into the original draft |
| Notifications | Persistent and UnifiedPush consumers implemented, end-to-end delivery unverified | Both modes: registration/recovery, real native attention, filtering, dedup and original-session routing |
| Routes and appearance | Saved hints, app/terminal appearance implemented, partly unverified | Persist settings and reconnect through supported controls without changing paired identity |
| Efficient updates | Event-driven transport; whole-history cache rewrites remain | Coalesced incremental persistence/publication, reduced offscreen/background work, immediate safety fences |

## Native and OS prerequisites

Diagnose the observed coarse-shell proof failure before attempting creation;
it occurs on the host before Android and is not an Android timeout. Capture only
the existing bounded opt-in first failing guard, preserve source and generation
fences, fix the cause and exercise that actual native path. Never submit to probe.

Interactive output and scoped closure need explicit authenticated native
contracts and matching consumers. Generic pane writes or group closure are not
substitutes. Design a separate least-privileged privileged component where safe
process provenance/atomic protected-subtree enforcement requires it. A high
reviewer must approve exact privileges before temporary owned-test activation;
no installed production service or global policy change is implied. Desktop
children retain zero controller capabilities. Use a trusted UnifiedPush
distributor app for scoped phone setup if needed.

Preserve exact Target/source/permission context, secure memory-only terminal and
content handling, finite limits, native receipts and once-only input. Reconcile
uncertain actions; never retry them or adopt resources from failed runs. Do not
weaken pairing, ownership, generation or permission policy to finish a feature.

## Delivery order

1. Functional component cleanup, scoped ignores and reviewed signed bulk baseline.
2. Persistent workspace/recent-agent browser and compact cards.
3. Chat/composer/activity presentation and stable truthful status.
4. Incremental updates and native creation blocker repair.
5. Live terminal/grid, modifiers and reviewed scoped lifecycle support.
6. Complete remaining matrix workflows, persistent delivery and UnifiedPush.
7. Targeted phone regressions, both-provider round trip and high review; deliver
   the usable tested development APK with precise remaining release work.

Authority: [UI](contracts/UI_DESIGN.md), [wire/state](contracts/PROTOCOL_STATE.md),
[security](contracts/PAIRING_SECURITY.md), [Herdr](contracts/HERDR_INTEGRATION.md),
[OMP](contracts/OMP_BACKEND.md), [Codex](contracts/CODEX_BACKEND.md),
[delivery](contracts/BACKGROUND_DELIVERY.md), [acceptance](ACCEPTANCE.md).
