# Herdr integration

Scope: supported plugin-v1 install/setup entrypoint, local topology discovery/recovery, durable native identity mapping and no-focus workspace creation.

[Documentation](../README.md) · [Protocol/state](PROTOCOL_STATE.md) · [Pairing/security](PAIRING_SECURITY.md#revocation-fence) · [Companion configuration](COMPANION_PACKAGING.md#configuration-and-trust-roots) · [OMP](OMP_BACKEND.md) · [Codex](CODEX_BACKEND.md) · [Source anchors](../REFERENCES.md#critical-files--anchors)

## Supported plugin-v1 entrypoint

Plan `plugins/goatr/herdr-plugin.toml` plus one executable Bash dispatcher `goatr.sh`; no new SDK, generic plugin layer, native GUI or Cargo package. Declare required `id = "goatr"`, name, a real release version, `min_herdr_version = "0.9.3"` and `platforms = ["linux"]`. Declare one action `install-setup` titled **Install / set up Goatr**, with argv `["bash", "goatr.sh", "open"]`, and one pane `installer`, `placement = "overlay"`, with argv `["bash", "goatr.sh", "pane"]`. Omit build/startup/events/link handlers entirely: registration and Herdr startup have no companion installation/start/pair side effects.

The `open` branch requires the injected `HERDR_BIN_PATH` and calls that exact running binary's `plugin pane open --plugin goatr --entrypoint installer --placement overlay --focus`; no guessed PATH binary, socket/session switch or daemon launch. The action does **only** this bridge and safe error reporting. Herdr captures action stdout/stderr, so prompts/setup/QR belong exclusively to the terminal `pane` branch; action invocation success means command launch, not successful install. Overlay is a supported interactive terminal and restores prior focus/zoom when closed; the flow must keep failures/results visible until explicit dismissal. [Packaging](COMPANION_PACKAGING.md#herdr-plugin-install-and-companion-lifecycle) owns every package/setup/update/removal effect.

Source baseline is Herdr `3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df`, whose Cargo version is0.9.3. Its minimum-version validation is **not** ABI/revision negotiation or proof every binary labelled0.9.3 includes this API. Before release/use, preflight the actual installed/running binary version/source provenance and real `plugin.list`, `plugin.action.list`/invoke and `plugin.pane.open` argv/environment/TTY behavior against this pin; absent/incompatible API fails before installation. Use pinned `docs/next` plus parser/schema/runtime/installer sources, not the stale versioned0.9.3 page's no-update/immediate-checkout-removal statements. Current source supports plugin update and leased checkout generations; new invocations use new files, running panes retain their old generation, and detached children are not protected beyond the parent/server lifetime. See [immutable evidence](../REFERENCES.md#herdr-plugin-and-nix-retention-evidence).

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

Before indexing `/proc/<SO_PEERCRED PID>`, prove that `/proc/self/stat` and the
first `/proc/self/status` NSpid match the caller's actual kernel PID, stably
before/after capture. A mismatched procfs view could otherwise make an equal
number identify an unrelated process. `ProcessEvidence` retains ordered NSpid
and PID/mount/user namespace object identities. Different provider namespaces
are not blanket incompatibility: use provable reporting-namespace/native-PID
joins and separately measured host/provider object views. Zero/inaccessible
peer PID or unprovable translation remains unavailable, never guessed.
`Client::process_info` returns public Herdr-reported PIDs as reported, not
silently reinterpreted host pathnames;5 supplies exact additional native joins.


Topology recovery is subscribe → wait `subscription_started` → fetch `session.snapshot` separately → replace. Events invalidate topology, not replayable mutations; serialize refreshes and repeat if invalidated while refreshing. `events_lost`, socket replacement/reconnect redo this sequence. No native snapshot/event global cursor exists. Coalesce invalidations, not identity checks.
Transient discovery failure cancels and awaits the affected read-only followers
before invalidating inventory. Successful discovery starts fresh subscribe →
`subscription_started` → snapshot recovery even when the healthy server is idle;
no topology event or native-effect replay is required.

The existing five-second discovery cadence also requests a coalesced
authoritative snapshot from each healthy subscription owner. Installed
Herdr0.9.1 can change its native-session/readiness report without emitting a
subscribed topology event; an idle event stream is therefore not proof that
the captured report remains current. This uses the existing cadence and
serialized recovery owner, not a provider-specific poll, new timer or upstream
runtime change. Structural snapshot equality includes native-session,
`launch_pending` and `interactive_ready` fields. An equal, current observation
from the same recovery revision/evidence/incarnation preserves its Source and
reducer state; an equal event-dirty or reconnected observation still installs
and revalidates the authoritative revision.

A silently changed observation claims its fetch revision with compare-exchange
and gates mutations through the existing `topology_synchronizing` reducer
before publishing replacement. A racing event discards that observation and
requires another fetch. A changed pane subscription set invalidates the old
Source before resubscription, then waits for the new ACK and fetches again;
the pre-resubscription snapshot is never installed. Pending cadence wakeups
survive that transition. Native path, owner/namespace/socket, registry/header
and final-snapshot provider guards are unchanged.


## Durable identity and routing

Persist a random serverId for the configured/discovered logical server slot (canonical socket location and named-server label under this host); change its incarnation UUID when Linux boot/PID-start/socket evidence changes. serverId is a UI grouping, not evidence that old terminal IDs remain valid. Collision of two live sources claiming a slot is ambiguous and fails closed.

Persistent session key is `(hostId,serverId,serverIncarnation,terminal_id,providerOwnerIdentity,nativeConversationId)`. Allocate a random sessionId once for a proven key; retain across companion/provider-connection restart only if all native identity evidence still matches. Provider owner means OMP instance/process identity or Codex owner process/socket epoch, not merely native conversation ID. New owner, terminal, native thread/conversation or Herdr incarnation creates a new sessionId; preserve the old row offline while cached history/receipts require it. A reconnect to the same owner/conversation or OMP room generation change keeps sessionId but replaces generation UUID before writes. Companion boot replaces all active generations/handles. Pane/workspace/tab movement only changes routing and does not change identity.

Join using explicit Herdr `agent_session` path/ID plus process/relay proof described in the backend topics. Never match cwd/title/latest activity. Herdr-supported OMP/Codex panes without sufficient backend proof produce protocol `UnboundRow` with terminal/routing/title and setup/ambiguity reason; they are not fake sessions with mutable IDs. A proven but disconnected binding retains its sessionId as offline. Unbound rows outside configured projects are still visible: projects restrict creation only.

The concrete topology owner shares the existing SQLite and serialized state
owners. `Storage::enroll_server` assigns only a bounded durable UI grouping to a
logical slot when source evidence is unavailable; absent evidence has **no**
trusted incarnation/session/creation authority. Validated `server_identity`
upgrades that same grouping, not a dummy binding. Persisted mappings are bounded
by64 servers/512 session identities, including retained offline identities.
`persisted_servers`/`persisted_sessions` enumerate them; restored display metadata
is offline with fresh boot generation, never native history/readiness proof.
Retirement requires the topology owner to release live/history obligations and
SQLite to independently release retained receipt/attention/read obligations.
Capacity exhaustion is visible, never silent retargeting or pruning.

Concrete5 inputs are `Config`/`ConfiguredProject`/`CodexOwner`, `Source` with
existing `ServerEvidence`/incarnation, public `Snapshot`/`Pane`/`Agent` native
reports and `PaneProcessInfo` plus `ProcessEvidence`. `Topology::begin_native`
allocates the proven durable identity and captures the actual reducer source
epoch while still synchronizing. Content is projected under that Target/epoch;
`finish_native` accepts only that exact key/owner/Target/epoch/server incarnation
and installs the final snapshot before readiness. Failed or late completion and
abort cannot consume a replacement installation. Provider callbacks retain both
Target and source epoch. Routing-only movement updates display without rotating
identity. Each native mutation resolves fresh authoritative topology and compares
the exact terminal/native report/incarnation plus owner/socket/file/root proof;
the old captured topology revision remains invalid. Dirty topology temporarily
marks the existing reducer Synchronizing, preserving native handles/dialogs and
authenticated reads until the same binding is reproved. Routing refresh merges
into current live state, not a stale status/activity snapshot. Actual source or
identity loss invalidates callbacks/handles and retains proven rows offline.
Unproven OMP/Codex records remain non-mutable `UnboundRow`, including those
outside configured projects. No provider-specific join/readiness is inferred.
Installing a different stable native key for the same server terminal first
offlines the superseded binding in the serialized commit. Its history, mapping
and receipts remain, but its source callbacks, handles and mutation readiness do
not survive merely because Herdr reports the same native session path/ID.

Source readiness has one checked implementation shared with strict `source` and
`Source::recheck`: absent/dirty authoritative revisions are explicit read-only
pending states, while physical I/O/identity failures remain errors. Public
signatures, error codes and the strict refresh-required message are unchanged.
The existing new-root wrapper proof uses the same distinction before and after
its unchanged kernel/object checks; a missing proof is not a refresh wait.
The real-source revision regression
`late_event_gate_revalidates_equal_current_revision_without_native_loss`
also covers pending → current without Target replacement, and missing/replaced
Unix socket objects remaining errors rather than pending readiness.


Shell-resolution measurements are local-only, tagged to the current proven
Herdr incarnation, exact action UUID/new root pane, actual shell PID/start and
expected **outer** wrapper. Explicit local `allow_scoped_shell_probe=true`
(omitted/default false) permits only the fixed procedure below; it is not
launch proof. Coarse actual shell executable/version/namespace and integration
object-view evidence admits an empty no-focus workspace attempt. Missing any
coarse prerequisite disables creation precisely, without filtering existing
sessions. Every later provider stage requires the matched result for that
action's new root; no old-pane/project-slot measurement can substitute.
Resolve provider paths from the evidenced process-root descriptor with Linux
[`openat2(O_PATH, RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS)`](https://man7.org/linux/man-pages/man2/openat2.2.html) and obtain object
identity from that returned descriptor. Absolute symlink targets stay inside
the provider root; an ordinary `/proc/<pid>/root/<path>` join is not this proof.
Unavailable kernel/path proof fails closed without executing a resolved child.


## Native desktop inventory projection

Existing `host.snapshot` / `host.changed` carries one `NativeHierarchy` per
proven server incarnation: flat native workspace/tab/pane arrays in returned
order, with actual opaque IDs/parents, numbers, labels, active tab, focus and
native agent status. serverId is Goatr's durable grouping alias; incarnation is
the proven process/socket epoch. Shell-only/unsupported and returned empty
workspaces are independent of supported-provider and creation-project filters.
No layout, cwd/environment, tokens, worktree/process reports or terminal output
is exposed. Pane label/native title/stripped terminal title/display-agent label
and agent alias stay distinct from the provider conversation title. Shared
bounded Label validation yields explicit displayUnavailable for unrepresentable
fields; restore diagnostics yield only restore_failed, never raw paths/reports.
See [wire types and finite limits](PROTOCOL_STATE.md#exact-object-vocabulary).

Only current graphs associate an exact proven Target or supported unboundRowId,
never both. The join requires current incarnation/terminal/native report and
existing binding/source proof; historical rows cannot attach by matching route
strings. Unbound aliases are incarnation-scoped. Dirty/source-lost graphs strip
associations; complete graphs become stale/source_unavailable, while display
overflow records remain unavailable with empty arrays and source_unavailable.
Both transitions emit host.changed even with no supported sessions.
Physical/native checks precede `State::replace_inventory`; its local
`association_epochs: &[(&Target, &Uuid)]` captures the existing binding's actual
provider epoch. Under the existing State mutex, exact Target/epoch, live
connectivity and routing checks prune expired Target and superseded unbound
associations. The validator checks only each captured Source's existing
invalidation atomic and server/incarnation, including unavailable/too_large
records: no topology/source lock acquisition, native I/O or new authority owner.
A late capture cannot restore source freshness or a disconnected provider's
association. This is display fencing only; public revision-sensitive
Source::recheck and native-effect authority remain unchanged.
Changed safe session routing is remembered through the existing SQLite display
owner; restore remains offline. Shared finite overflow rules use too_large only
for a confirmed current source, preserve source_unavailable otherwise, and keep
supported inventory/control where its own budget fits.

These fields use the inspected Herdr0.9.1/protocol22 source
`/nix/store/pl77i20fjwgvkmd2pc7qyxzq5nkghy7a-source`, distinct from the plugin
pin. Running-source equality and live host/provider mirroring remain unobserved.
Focus decorates only: no phone retarget/open or desktop-focus mutation. OMP's
selected branch stays unknown; Codex same-owner proof remains separate. No new
method/subscription, arbitrary shell/pane control or second tree owner is added.

The existing `late_inventory_capture_cannot_reattach_disconnected_provider` and
`subscribed_shell_node_overflow_invalidates_display_and_fences_late_capture`
OMP regressions exercise these commit/event boundaries. After-repair execution,
shared gates and installed/live mirroring checks remain coordinator-owned and
requires coordinator validation. LSP is unavailable; the single API's producer,
capacity test, three captured-commit tests and smoke-review caller are migrated
by complete source references, with no compatibility variant.


## Atomic visible-terminal reads

For a bound OMP/Codex Target, `pane.read_bound` accepts exactly
`{pane_id,expected_terminal_id,expected_read_identity}`, where read identity is
`{herdr_incarnation,runtime_epoch,binding_epoch}`. A single native actor admission
checks the expected pane, terminal and nonreused identities before reading visible
ANSI-stripped text. The result echoes pane, terminal, workspace, tab and read
identity with `text` and explicit `truncated`, bounded to16KiB UTF-8.
No focus, input, native scrolling or second runtime is involved.

Pin the optional native identity when installing the original binding. Runtime,
native-binding or incarnation replacement uses existing source/generation
invalidation; never replace the pin beneath an existing Target. Missing identity,
epoch exhaustion and older native APIs leave terminal-read capability false.
Validate every echoed identity and the original source again before admitting
text. Use the native lossless JSON parser for u64 epochs; phone Uint conversion
is not valid. Ordinary `pane.read` cannot substitute for this atomic guard.

This slice grants no target or write permission to unbound panes or shells.
Their broader terminal controls require separate native authority and no-replay
contracts. Goatr’s wire and lifecycle rules are in
[bounded terminal reads](PROTOCOL_STATE.md#bounded-terminal-reads).

## Bound ordinary-shell foundation (planned)

This separate full-parity slice provides shell inventory, visible-text reads and
**New terminal in this terminal’s directory**. It grants no input or close.
Goatr consumers and phone workflows remain incomplete until their own cutover.

Native `terminal.mobile_shell_access` is `off` by default; an explicit local
`read_create` grant is required. Positive process-local provenance comes only
from an actual ordinary-shell spawn and follows that exact runtime into the
registry. Absent agent metadata, restored/imported runtimes and command factories
are not shell proof. Provider/hook acquisition, managed-agent start, handoff or
runtime replacement invalidates authority without revival when metadata clears.

Existing native inventory may include optional shell scope and independent
read/create/input/close flags. Scope pins pane, terminal, workspace, tab, existing
read identity and permission epoch. Input/close stay false. Missing support or
grant means false, never generic pane fallback. Native `shell.read_bound` admits
that exact scope and retains the existing16KiB UTF-8/private-text boundary.

Native `shell.create_bound` takes only operation UUID and exact scope. Capture
and revalidate the original shell's authoritative process-derived current cwd
and directory identity across waits; mutable cached cwd alone is insufficient.
Use the ordinary configured shell/environment, no phone path/command/environment
or focus/trust override. Agent-process directory creation uses the separate
[bound provider process seam](#agent-process-directory-terminal-creation-planned);
ordinary shell provenance is never substituted.

Share bound creation's original control, absolute deadline,4096-operation limit,
exactly-once settlement, monotone typed facts and single late-factory holding
slot. Worktree and shell kinds have separate source validators/results; a kind
mismatch invalidates rather than replacing the ledger. Reserve identities,
construct the detached tab, record produced identities and re-admit before actor
insertion. Expiry never releases a still-running factory reservation; late or
uncertain resources are retained without insertion, deletion or implicit retry.
The existing private workspace bypass remains worktree-only. Later facts may
enrich the retained ledger but never rewrite its terminal response/receipt.

Interactive input still needs consume/flush-time native permission and partial
write accounting. Close needs identity-safe descendant shutdown; generic pane
close and bare PID enumeration do not establish that authority.

## Guarded ordinary-shell input (planned)

The next native slice adds explicit local `read_create_input`; existing `off`
and `read_create` grants never acquire input. Permission changes rotate the
existing epoch. Only a positively admitted original ordinary-shell runtime may
advertise input; close remains false. No installed configuration is changed by
this source implementation.

`shell.input_bound` accepts operation UUID, exact BoundShellScope and one closed
text/keys item. Text is1–16384 UTF-8 bytes with no controls except LF/tab; keys
are1–32 typed events using the existing encoder, with unsupported modifier
combinations refused. Text adds no Enter; keys never include the draft.

One guarded PTY queue item carries the original runtime, absolute5-second native
deadline and monotone encoded/written-byte counts. Runtime-owned authority gates
serialize every actual nonblocking write with revocation. Metadata remains pure
data: owner transactions revoke before publishing changed scope/eligibility;
child completion, removal, handoff and permission loss revoke before effect.
Release gates across waits, polls and flush waits. Never resolve a replacement
runtime or reactivate an old queued item.

The result echoes operation/scope and accepted, refused_before_effect or
uncertain, with bounded byte progress and flush completion. Accepted means the
whole item crossed the existing flush boundary, never command completion.
Partial/lost/unproved delivery stays uncertain; deadlines stop remaining bytes.
No suffix retry, cleanup keys or automatic replay. Goatr input consumers require
a separate canonical wire/receipt and original-viewer admission cutover; current
builds still expose no input.

## Atomic Worktrees reads

Planned native `worktree.list_bound` accepts
`{pane_id,expected_terminal_id,expected_read_identity}`; no caller cwd, repository
or trust override. An optional-false `session.snapshot.worktree_list_bound` flag
explicitly advertises support independently of terminal reads.

Admit original pane, terminal and complete read identity under the native actor,
derive checkout/hierarchy from authoritative native state, and capture repository
and common-Git identity. Run fixed read-only Git operations outside the actor,
then revalidate native and repository identity before releasing any result.
Echo original pane/terminal/workspace/tab/read identity; Goatr verifies all fields.
Do not substitute unbound `worktree.list`, a shell command or a terminal read.

The total operation budget is5 seconds including resolution, with256KiB aggregate
captured stdout, at most128 entries, paths4096 bytes and labels/branch/head1024
bytes. Discard stderr; bound/drain output, and kill/reap only the operation's owned
child on expiry/overflow. Preserve Git trust and disable external helpers,
textconv, fsmonitor and optional index refresh. No shell, config changes or new
engine. Encoding, bounds and replacement failures return fixed safe errors,
without partial results or raw stderr. Inventory is an observation, not an atomic
working-tree snapshot. Linked-worktree status may be absent when unproven.
Final actor publication rechecks captured directory device/inode metadata after
off-actor resolution, while the independent watchdog retains response ownership.
These metadata calls can stall the actor on a slow filesystem; the watchdog bounds
request settlement, not actor responsiveness. Prepare serialization before claiming
the response, check the original deadline on both sides of that claim, and publish
without intervening I/O or serialization.

## Bound Worktree management (planned)

Accepted implementation contract; these mutations are not implemented or
advertised yet. Cut over schema, Rust/Kotlin consumers and fixtures together.
Ordinary Herdr worktree mutations and inventory paths confer no mobile authority.

Extend receipt-backed session.create with all-or-none sourceTarget and checkout:
new checkout has kind=new, optional branch/base; existing has kind=existing,
required branch and forbids base. Branch/ref bounds are 1..1024 UTF-8 bytes, no
NUL/control; native Git validates semantics. Persist a generated action-UUID
branch and requested base (default HEAD) before dispatch. New rejects existing
branch/path; Existing requires an available local branch and never resets,
forces, detaches or adopts. Both create one linked checkout and launch there.

Optional-false worktreeCreate and worktrees.list creationScope expose this only
when the bound repository/common-Git identity maps uniquely to an authorized
configured project. Companion enforces that mapping; native Herdr reads no Goatr
config. Phone input never includes cwd/path/repository/focus/trust authority.

Native worktree.create_bound admits exact original pane, terminal and read scope
under its actor, reserves repository/branch/operation against competing changes,
resolves an immutable commit, and derives the path from its native worktree root.
Use fixed argv/native helpers with focus/trust false. A correlated result retains
kind, branch, requested base, resolved 40/64-hex commit and every known checkout,
workspace/tab/root-pane fact. Only proven pre-effect refusal is no-effect Failed.
Partial Git effects, lost replies, timeout or final identity mismatch are
Uncertain with known facts retained; never retry, adopt or clean up implicitly.

The independent watchdog settles by the original deadline even if the actor or
factory stalls. Settlement does not release an unresolved operation reservation
or permit late workspace insertion. Retain late factory objects and monotone
facts in one operation-bound private holding slot; while occupied, refuse another
bound creation. No adoption, implicit object cleanup or rewrite of a terminal
receipt. Retain later facts locally under the original operation correlation.

Bound mobile creation disables hooks, fsmonitor and submodule recursion only for
its Git processes; it changes no global/repository configuration. It does not
claim configured post-checkout hook setup. Executable checkout filters require a
proven pre-effect refusal rather than an unbounded helper launch. Owned process
group cleanup is not containment of arbitrary detached helper descendants.

Expected creation events may change global topology revision. Native completion
correlates the admitted original pane/terminal/repository and operation, without
requiring the old global revision after its own effect. Subsequent stages retain
all existing source/Target-generation fences; successful creation grants no
permission to continue launch after source invalidation.

CreationProgress retains sourceTarget; monotone resources.checkout retains kind,
branch, requestedBase for New and resolvedCommit once known. Exact paths/object
identities/common-Git proof remain in durable host-local creation metadata. One
shared execution-checkout proof feeds shell validation, OMP --cwd and Codex
thread/start.cwd. Configured project remains authorization scope; shared Codex
owner policy and owner working directory remain unchanged.

Secure Worktrees creation offers New/Existing, provider/name/branch and Base only
for New; blank New branch is generated and blank Base resolves HEAD at creation.
Capture exact host/source/Target/ticket and revalidate foreground, mutationReady
and creationScope at submit/result. One submission owns one durable action;
process death/reconnect resumes its receipt without native replay.

Removal is a separate required slice with optional-false worktreeRemove. Native
bound inventory mints opaque candidateId values for positively registered linked
siblings in the original common-Git repository, including closed/unmapped
checkouts. Optional managedWorkspaceId is occupancy/display metadata, never
selection authority. Requiring a live workspace would make safe removal
unreachable: its last terminal either respawns or removes the workspace.

Bound remove_preview selects candidateId and returns a short-lived one-shot
previewId plus native path/branch/head and a closed refusal reason. Retain at most
64 previews for60 seconds, bound to the original scope, exact checkout/admin/.git
objects, HEAD/branch and occupancy. Public receipt-backed worktrees.remove accepts
bootId/target/candidateId/previewId; phone paths, force and trust are forbidden.
Use the existing single mutation reservation and finite action deduplication;
consume previews at admission. Gate competing native launch/create/open/move or
respawn into the reserved checkout. Preserve the original actor/source and the
5-second effect budget through queue, Git and completion.

Refuse source/current/main/bare/locked/prunable/missing checkouts, changed identity,
known native terminal/agent occupancy including nested paths, dirty/untracked or
ignored contents, submodules and masked index entries. Use bounded fixed status
and ls-files reads; one pinned non-force Git worktree remove is the only effect.
Its typed guarded execution policy may permit the exact internal pinned-Git
status subprocess after validating argv/environment/cwd/image before execution;
all other helper execution remains denied. Existing Changes policy is unchanged.
No generic deferred-removal completion: keep all workspaces/tabs/panes/providers
untouched. Do not delete branches, prune, close, kill, reset, revert or clean up.

Retain monotone effectStarted/gitRemoveSucceeded/checkoutAbsent/registrationAbsent
facts before waits. Success requires zero exit, checkout and registration absence,
and original scope validity. Proven pre-effect refusal fails without effect;
every ambiguous or nonzero result after effect start is Uncertain. Watchdog
settles once; unresolved cleanup retains the reservation, with no late topology
mutation or retry. Bounds remain256KiB aggregate,128 entries,4096-byte paths and
1024-byte labels. Observed cleanliness and native occupancy do not exclude all
external writers; confirmation tells users to stop other tools first.

Reachable flow: close target terminals on desktop, keep the original source
agent open, refresh Worktrees, preview the closed linked checkout and explicitly
confirm removal. No automatic terminal closure or replacement session.

## Atomic Changes reads

Planned read-only slice: reuse the bounded repository, scope and publication
machinery above, with an independent capability flag and method. One original
5-second deadline and256KiB aggregate stdout cover discovery, every repository,
status, staged/unstaged numstat and patches, and revalidation. No partial result.

Changes uses a Linux-only guarded Git runner and an explicit packaging-owned
immutable Git launcher/ELF identity. A dedicated tracing worker owns spawn,
nonblocking output, trace events and cleanup. Its exclusive process group and
TRACEME→initial exec stop precede launcher/Git user execution. Verify each admitted
image; permit only the supported pinned original-leader launcher→Git transition.
Scrub child-only shell/loader injection; preserve native/provider permission policy.

After actual Git admission, deny every execve/execveat entry before resuming it,
including all descendants. A denied helper poisons the whole observation even if
Git would otherwise recover from an optional filter failure. Do not clear filters
or treat configuration enumeration/postchecks as an execution guarantee. Trace
fork/vfork/clone/exec/exit with EXITKILL and syscall stops; reject unsupported ABIs,
clone3, untraced/namespace/parent-changing clone flags and process-group/session
escapes. Use a finite tracked-task bound; never reap unrelated native children.

The leader remains stopped/unreaped until owned group cleanup is issued on every
outcome, including success. The tracing worker services termination itself; never
transfer active trace handling to the generic reaper or detach unguarded tasks.
The watchdog still settles the client by the original deadline if cleanup stalls.
Missing pins or denied/unsupported kernel tracing fail closed. Do not add
capabilities or change Yama/LSM/container policy. This guards fixed Git execution;
it is not a general filesystem sandbox or an agent permission bypass.

Root observations use porcelain-v2 status -z --branch --untracked-files=all,
staged/unstaged diff --numstat -z, and patches with --no-color --no-ext-diff
--no-textconv --submodule=short; use --cached for staged and one consistent explicit
rename setting. Use --ignore-submodules=dirty to retain gitlink commit changes
without Git recursively executing status. Disable optional locks/index refresh,
fsmonitor and lazy fetching; preserve trust. No shell, caller paths/options,
binary patch payloads or recursive submodule patch contents.

Discover gitlinks from fixed ls-files --stage -z. Resolve initialized repositories
explicitly and require their checkout to equal the captured gitlink directory,
never a parent fallback. Recurse guarded status observations; compose tracked/
untracked dirtiness, including nested submodules, and preserve native commit-change
information. Add a derived tracked submodule entry when dirtiness was omitted
from root status. Bound repositories, reject cycles/ambiguous gitlinks, capture
and revalidate every participating repository identity. No submodule file contents.

Parse ordinary, rename/copy, unmerged and untracked records explicitly. Numstat
joins are side-specific; historical rename sources do not reserve current paths.
For a status-proven conflict, recognize only the documented unmerged zero-count
sentinel plus comparison; reject arbitrary duplicate or ambiguous records. Binary
flags reflect actual numstat markers. Discard stderr. Commands are separate
observations, not an atomic working-tree snapshot. Preserve final metadata/CAS
checks and their actor-latency limitation. Sources: [Git formats](../REFERENCES.md#bounded-repository-reads)
and [Linux execution fence](../REFERENCES.md#changes-execution-fence).

## Atomic workspace rename

The native optional-false snapshot flag `workspace_rename_bound` maps to Goatr
`workspaceRename`. Native `workspace.rename_bound` accepts pane_id,
expected_terminal_id, expected_read_identity and label, without an independent
workspace selector. Under the existing actor lock, resolve the original pane’s
workspace, compare the complete original runtime/binding identity immediately
before mutation, then reuse normal rename, save and workspace.renamed publication.
Return the bound scope and resulting label; the companion validates that reply
and retains only Target/workspace identity in its receipt. Apply the existing
protocol Label bound on both native and companion inputs.

No repository resolution, terminal input, provider command, new session, focus
change or authority fallback is part of rename. Ambiguous writes stay uncertain;
known native success remains an acknowledged effect of the original action.

## Workspace creation

The public kind launcher accepts `{name,kind,pane_id,args,timeout_ms}`, obtains `interactive_agent_executable(kind)` and submits shell-encoded argv; it has **no executable parameter**. The user-approved exception permits one fixed read-only resolution query through public `pane.send_input` **only** in a fresh Goatr-created idle no-focus root pane. It is never native conversation control. Preserve installed wrappers, user shell selection, signing and namespaces; do not run a raw wrapper child or provider `--version`, alter aliases/functions/PATH/startup files, autoload a resolver or force a provider transition. Writable setup8 separately discloses/collects local consent; no synthetic Authorized peer is used for local setup.

Create `workspace.create {cwd:<canonical configured project>,label:<name>,focus:false}`; retain returned workspace/root pane and resolve its terminal_id via authoritative snapshot. Validate project availability/wrapper access at dispatch. Name uses protocol label bounds. The backend correlation alias is `goatr-<provider>-<first20 action UUID hex>` (hyphens removed from the UUID), at most32 characters, matching Herdr's `[a-z][a-z0-9_-]{0,31}` constraint. Persist the full action UUID and alias before effects; reject an alias collision rather than reuse another action's agent. The alias is never shell syntax. Server must already exist; creation never starts a different named Herdr server.

Use [CreationProgress and CreationResources](PROTOCOL_STATE.md#ordered-creation-progress), with intent persisted before each side effect and returned IDs before the next. Capture workspace/root pane/terminal, owner start if needed, native thread/session, relay binding and final ready Target. Native readiness includes the exact desktop terminal, not only a running helper. After a lost response, read-only reconciliation may identify the resource by recorded correlation; ambiguous effects stay uncertain. Partial workspace/thread/relay resources remain visible in action.status/UI, never implicitly retried, deleted or rolled back. Revocation fences each subsequent stage without killing earlier desktop work.

### Once-only new-root shell probe

`Topology::shell_probe_once(security,auth,action,params,deadline)` follows the
already-confirmed workspace response and authoritative created-root snapshot.
The source/incarnation/reporter, exact workspace/pane/terminal, unchanged
desktop focus, no managed/native agent or launch pending, and shell-only
foreground predicate are checked before admission and after completion.
`reported_process` joins only independently evidenced namespace/PID objects;
`shell_executable` compares the kernel-opened ELF executable to trusted
host-root and process-root object views. Source-set admission is finite:
`/nix/store/<hash>-{bash-5.3p9,bash-interactive-5.3p9}/bin/bash` or
`/nix/store/<hash>-{zsh-5.9,zsh-5.9.1}/bin/zsh`, not a package substring,
arbitrary renamed ELF or wildcard version. GNU Bash checks runtime major/minor/
patch exactly5/3/9 (`type -t` then inherited cached `type -p`); zsh checks
exactly5.9 **or**5.9.1 (`whence -w`, ordinary external/hashed path).
Both preserve inherited hash/PATH precedence rather than fresh PATH-only
search. Unproved dialects remain disabled, never an inferred POSIX fallback.
Target alias/function/builtin/autoload, wrong outer path and helper/trace
rejection checks remain. These package names are inspected-source admission
metadata, not executable-byte attestation or proof of Herdr's configured shell.
Actual new-root shell/process/source and result proof is still required.

If the initial exact root-pane process snapshot is specifically `ShellNotIdle`,
creation may perform a read-only 200ms-cadence wait only to the original
creation deadline. It pins the original reporter namespace, shell PID/process
evidence, supported executable and dialect; each iteration rechecks
authorization, the original source/root lineage and exact pane/focus, pinned
shell epoch/executable, and same-evidence `process_info`. Any other initial or
later guard remains terminal. No artifact, nonce, query intent or input exists
until that exact original shell becomes idle; persistent non-idle settles as
`shell_not_idle`/Timeout without extending the deadline.
A successful query result can be published before its original shell is idle.
Only this post-result `ShellNotIdle` condition may use the same deadline-clipped
200ms status wait. Pin the original source/root/focus, reported PID, full shell
and reporter process evidence, executable and dialect; recheck authorization and
identity around each bounded await and before accepting idle. Validate the pinned
host process directly rather than rediscovering its namespace on each checkpoint.
All other failures remain terminal. Do not reread the result, resend the query,
recreate its intent or extend the original creation budget. Artifact recheck and
final authorization/receipt settlement still follow the successful idle proof.

At measurement time, the expected outer wrapper may use a different path
spelling only when the resolved wrapper and configured wrapper pass shell `-ef`
file-object equality. This is neither path nor executable-byte equality, and it
does not prove an alias cannot retarget before launch; existing process-root,
shell-epoch, stamp and final launch fences remain required.

The approved Bash variants share GNU5.3 archive SHA256
`0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba`,
patches `bash53-001` through `009` and local `pgrp-pipe-5.patch`; interactive
readline packaging is not another dialect. No p15/static/5.3* admission follows.
The retained zsh5.9.1 archive SHA256
`5d20bec03f981dc4e9a09ec245e7415388ff641f79c5c5c416b5042e58d8280d`
was realized and inspected in its actual resolver/PID/options/core-helper
members. Those relevant branches agree with the inspected5.9 baseline and
[immutable5.9.1 source](https://github.com/zsh-users/zsh/tree/0e0d4ea11731c47f57bad042fbe75e3979d8a1d2/Src);
the same inherited-subshell query applies, not a new resolution algorithm.
This is relevant-source evidence, not whole-release or executable-byte equality,
an executed shell-check pass, or live Herdr-shell proof.

One exclusive0700 attempt directory and precreated0600 single-link result file
under the integration root retain their open FD/dev/inode. Shell-visible
parent/file objects must match through the existing process-root openat2 proof.
The single control-free command and one complete result are each at most8KiB.
Dynamic literals are only fixed provider kind and locally validated/quoted
expected outer path/result path/32-byte nonce. The record contains only
version/nonce/original shell-native PID/finite category/match/completion.
No alias/function body, PATH/environment, stderr, raw resolution dump or
native payload is published or stored in receipt metadata.

Host-local `WorkspaceOperation::ShellProbe` and `ShellProbeMetadata` add one
distinct intent/ticket/ACK/result subrecord to existing creation storage;
the wire stage remains Workspace and retains already-confirmed root IDs.
Tickets include operation/attempt, so old workspace ACKs cannot settle a probe.
Every provider-stage intent waits for this action/root's confirmed subrecord.
The actual connected Herdr FD fences exactly
`pane.send_input {pane_id,text,keys:["Enter"]}` with correlated `result.type:"ok"`.
That ACK proves queue admission only. Result polling is read-only outside the
10-second boundary and continues only to the original creation job's absolute
deadline (OMP120 seconds; Codex300 seconds), never a refreshed or additional
five-second cutoff. Expiry is checked before each poll, accepting a completed
result and final proof validation; an expired observation reaches normal
settlement and cleanup through the existing receipt path.
Only nonce/PID/epoch/object/focus/idle/path matching installs the exact-pane
`ShellMeasurement`. Lost, malformed, replaced or missing outcomes remain
uncertain; known rejected resolution can fail while retaining the workspace.
Revoke and restart never re-enqueue, relaunch, roll back or consume late proof.

After durable probe or launch intent, synchronous admission reads the current
authoritative topology revision for the original server, requiring unchanged
incarnation, process/socket evidence, socket path and subscription invalidation
lineage. An unrelated completed topology refresh does not invalidate that
original creation. Admission rechecks the exact retained workspace/pane/terminal,
desktop focus and absence of agent/launch activity against this current snapshot;
the original shell/executable and probe result-object checks remain mandatory.
Dirty, missing or replaced sources fail closed immediately. This adds no wait,
new intent, connection, probe, launch, deadline extension or effect replay.

A rejected shell-probe admission preserves its closed guard in the existing
single failed-stage settlement: `shell_probe_root_changed_before_enqueue`,
`shell_probe_shell_epoch_changed_before_enqueue`, or
`shell_probe_result_view_changed_before_enqueue`. Keep the original ErrorCode
and resources; these proofs contain no native values and authorize no retry.
Other creation-boundary callers retain their generic admission proof.

The deployed protocol22 API has no atomic foreground/PID compare-and-enqueue
or queue recall. Same-user interference/startup/DEBUG/preexec/helper shadowing
can race or observe typed input; native FD shutdown does not retract admitted
PTY bytes. In particular, quoted initial `'builtin'` bypasses aliases, not a
function named `builtin`; the core whence/printf paths are source-proved, but
their first helper token does not bootstrap arbitrary ambient-helper trust.
Do not infer unconditional helper-shadow/no-autoload safety from package
admission or weaken rejection checks. Queries run in an inherited subshell so
resolver hash changes do not modify the parent and these fixed queries do not
write a disk cache, but this is not a universal custom-shell safety/history
guarantee. The nonce is host-local stale-result correlation, not authentication
against another same-UID process; text may be echoed/history-retained. Detected
interference fails closed, with no focus repair or global shell edits.


`Topology::workspace_once` and `launch_once` use the retained receipt alias,
existing security creation fence and owned duplicate of the **actual** effect
socket. Workspace response IDs are persisted immediately; separate authoritative
snapshot confirmation occurs outside the10-second write fence and before any
next effect. A changed source/malformed or lost response retains validated known
resources as uncertain. A server must already have an active workspace: deployed
Herdr0.9.1 otherwise focuses its first workspace despite `focus:false`.
Read-only readiness never holds a write/start fence.

A correlated `agent.start` submission ACK may retain Launch/Dispatching while
the original in-process job obtains exact native identity read-only outside the
fence. It is not native readiness, does not permit the next stage, and cannot
authorize another launch. `creation_result` confirms Launch only after the
existing complete resource/native proof requirements hold. Revocation before or
after ACK settlement, or companion restart, retains this unconfirmed effect as
uncertain; it never becomes a fabricated no-effect failure or automatic retry.
Only Ready/Confirmed and the exact resulting binding can yield succeeded.
When revocation occurs between already-confirmed stages, fail the remaining job
with unauthorized while retaining every confirmed resource. A genuinely
Ready/Confirmed binding preserves the exact succeeded created result. Unknown
entered stages remain owned by their existing dispatch ticket; no guessed
failure, authenticated bypass or new ticket/job framework is introduced.
A definitively rejected, already-settled failed stage also terminalizes as
failed on revoke, retaining its exact result/proof and all earlier resources.
It cannot remain dispatching after its entered ticket has been consumed.

Codex's concrete creation consumer uses the same workspace/probe/launch records.
`Topology::codex_availability` reports its independent owner/service prerequisite;
canCreate is still attempt eligibility, not native Ready. After exact TUI binding,
`validate_created_binding(action,params,resources)` rechecks only that original
action's measured pane/shell/project/outer-wrapper and current terminal/root:
it does not require an already-running TUI to remain an idle shell and cannot
admit another provider launch. Ready additionally needs the immutable Codex
owner/socket/publisher proof, exact TUI-correlated resume T and current loaded
membership. Native routing refresh preserves the existing Target/source epoch;
no focus, interpreter, PATH/rc, existing-pane probe or hierarchy authority changes.

A scoped real-TTY Codex run exercised these original workspace/probe/launch
records through native Ready and successful phone history/tool-marker reads on
one immutable Target. Its genuine desktop barrier remains unacknowledged and
the owned helper/native work is retained. This is not plugin lifecycle, desktop
interaction or final combined-provider Android acceptance; those gates remain
for the clean integrated artifact.





## Acceptance gates

See [Herdr plugin lifecycle](../ACCEPTANCE.md#herdr-plugin-installation-and-lifecycle), [live session and creation](../ACCEPTANCE.md#live-session-and-creation), [generation replacement](../ACCEPTANCE.md#reconnect-and-generation-replacement), [mutation receipts and restart](../ACCEPTANCE.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.

## Planned bound agent alias rename

Advertise optional-false `agent_rename_bound` independently from generic rename.
`agent.rename_bound {pane_id,expected_terminal_id,expected_read_identity,name}`
pins the existing bound worktree scope, and verifies the original direct pane
still hosts the same non-pending agent/runtime before one same-actor alias
mutation. Required alias follows `[a-z][a-z0-9_-]{0,31}`; use existing uniqueness,
save and publication mechanics. Return `{scope,name}`. Do not focus, launch,
split, write terminal input, resolve a new alias or change a workspace/group.
Goatr separately joins original PaneInfo.agent to the provider Target at capture,
Security admission and socket fence, preserving source/socket/incarnation/epoch/
read-identity/generation/permission guards. Generic agent.rename is not a fallback.

## Planned attachment staging visibility

Companion-owned upload staging is not a native terminal/file-write RPC. Retain
the original provider binding's host↔agent runtime-directory object/namespace
proof, following the existing ProbeArtifact/process_object ownership pattern
without issuing a shell probe. Validate each staged object through that original
process proof before publishing its derived provider path. A visible file remains
subject to normal native provider sandbox and approval policy. No generic path
write, current-directory inference, replacement-process adoption or shell input.

## Agent process directory terminal creation (planned)

Native shell.create_agent_bound is separate from ordinary shell.create_bound. Its host-only BoundAgentShellScope pins the exact provider pane/terminal/workspace/tab/read identity, the existing mobile-shell permission epoch and original provider process proof. OMP uses its validated Connection.owner; Codex uses validated Presence.tui. AgentInfo.cwd, foreground_cwd, labels, cached paths and logical thread cwd never supply authority.

Linux retains the original process proc directory and proves boot/start ticks, UID, PID/mount/user namespaces and namespace PID chain. A bounded, revalidated ancestry join (at most128 processes) reaches the exact pane child and current foreground job. The same original process cwd path and directory dev/inode are re-observed before and after waits. Missing proof, non-Linux support or the default-off local grant refuses creation. Native snapshot advertises agent_shell_create_bound plus optional agent_shell_permission_epoch using the existing permission counter only.

A five-second native-owned deadline starts at typed socket admission. The AgentShell kind shares the existing consumed4096 UUID set, actor/control gate, monotone ledger, watchdog and single late-factory slot. It constructs one unfocused ordinary configured shell, retains all generated identities before waits, and installs only after original scope/process/cwd and created child directory revalidation. Post-effect loss is Uncertain with retained facts and no cleanup/retry. Close and provider input authority do not change.

## Bound artifact browsing (planned)

Native `terminal.mobile_artifact_access=off|read` is independent and defaults off. Its own monotone permission epoch disables on exhaustion. Snapshot artifact_read_bound/optional artifact_permission_epoch are native-only. `artifact.list_bound/open_bound/read_bound/close_bound` carry exact original pane/terminal/workspace/tab/readIdentity, artifact epoch and either retained original provider-process proof or positive ordinary-shell provenance. Provider OS cwd is OMP's original owner or Codex's original TUI; ordinary-shell cwd comes from its retained spawned child. No cached, logical, foreground-label or phone cwd supplies authority.

Independent optional `PaneInfo.artifact_shell_scope` permits Files without granting terminal read/create/input. Existing canonical ShellTarget mapping owns both optional shell and artifact proofs; no second resource registry or invented shell permission epoch. Legacy persisted shell proof is decoded without changing UUIDs or generations; ordinary source/proof rotation remains separate policy.

The Linux resolver pins the actual cwd directory object and mount, uses descriptor-relative openat2 BENEATH/NO_MAGICLINKS/NO_SYMLINKS/NO_XDEV, and fails closed if unavailable. Descendants are first pinned O_PATH and proven regular/directory before readable reopening of that retained exact FD through verified procfs; special nodes never receive a readable open. No canonical-path or path-prefix escape fallback exists. Registered refs contain native-only path components. Opening snapshots exact bounded regular-file bytes into native memory and compares device/inode/type/size/mtime/ctime/mount and original source before/after. An FD alone is never described as immutable. Non-Linux artifact capability is unavailable.

Native global bounds are independent of opaque caller owner labels. The companion separately enforces authenticated-peer quotas. Source/backend Arc, physical socket/evidence, generation, readIdentity, permission and process proof remain original through publication. Config/source/cwd changes invalidate reads; no polling, provider commands, file writes or source substitution. Exact known memory-lease close may clean up after read authority is lost, with original owner/correlation and original physical server only. Generated native schema and actual resolver compatibility remain coordinator verification work.

Artifact reads retain original physical source, invalidation owner, backend connection, Target/generation/source epoch, scope, grant and cwd/ref proofs. A newer authoritative Ready inventory revision may advance without replacing those proofs; pending inventory permits no reads. Explicit root refresh re-enumerates natively and replaces only that exact owner’s catalogue, retaining existing immutable reads.

## Independent OMP Skills preview scope

`terminal.mobile_skills_preview=false` is a separate local USER-read grant, never shell-input authority or model tool approval. Grant changes rotate the existing feature's monotone permission epoch; overflow disables it. Snapshot `omp_skills_preview_bound` and `omp_skills_permission_epoch` are optional false/None when unsupported. Linux support requires the retained original process/pane join.

Native `skills.scope_bound` accepts a native-only exact pane/terminal/workspace/tab/readIdentity, permission epoch and full original OMP process proof. Its admission deadline is stamped once at typed request decode. It rechecks grant, exact agent binding and retained original process-to-pane job proof around waits, then echoes the original scope. It reads no document, starts no process and changes no native identity. Native process/permission facts stay host-local. The companion additionally pins the original physical socket/incarnation/invalidation owner and provider Arc through guarded response publication. Generated native schema must be produced by the coordinator before packaging.
