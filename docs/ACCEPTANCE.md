# Testing and acceptance

Scope: the proposed implementation command suite, owned smoke resources and all required behavior/evidence gates. Each gate retains the original required check.

[Documentation](README.md) · [Dependency order](README.md#dependency-order) · [Tech stack](contracts/TECH_STACK.md) · [Source anchors](REFERENCES.md#critical-files--anchors)

Current commands and gate owners are below; implementation/verification status,
exact observed commands, source/artifact identities and blockers belong in the
[status](STATUS.md). A configured command,
fixture filename or source inspection is not a passing runtime gate.

## First testable APK UI handoff

The owner permits a development APK for immediate feedback with known failures
and unverified behavior disclosed. Use incremental builds and checks limited to
changed behavior. Full clean builds, exhaustive acceptance and completion of
this checklist are deferred to a later exercised checkpoint, not waived or
passed. Identity, permission, privacy and no-replay protections remain required.

The approved [complete Compose interface](UI_POLISH_PLAN.md) is the
current presentation goal and remains tracked checkpoint work.
The checklist below does not block development feedback and does not claim a pass:

- Phase 0: reviewed design tokens, component gallery, phone/wide-window layouts,
  state inventory and copy for Ready, Working, Needs input, Offline, Changed
  session, uncertain send and large-text states.
- Phase 1: polished app bar and session browser/switcher, readable transcript
  and tools, request dock, composer and action details, with actual prompt,
  Stop, forms, receipts and recovery behavior preserved.
- Visual quality review using screenshots of the actual owned APK: normal and
  200% fonts, 320/360 dp compact layouts, landscape, IME and wider windows where
  exercised. Inspect readable content, reachable actions, wrapping and privacy;
  identify each source/APK, owned resource and actual configuration. Synthetic
  galleries remain design examples, not actual-provider screenshots.
- Functional evidence remains required for keyboard/composer operation, stable
  reading/selection, original-text Copy, slow/uncertain sends, double taps,
  request closure, rotation/process death and generation replacement without
  automatic replay or retargeting. Screenshots do not establish these effects.
- Matched source/APK identities, exact checks, safe representative screens and
  actual behavior evidence, alongside the existing real OMP/Codex foreground,
  identity, receipt, privacy, recovery and once-only gates. Screenshots and
  Compose previews alone do not satisfy native behavior.

The owner explicitly defers the dedicated accessibility/TalkBack audit and a
new release-like comparative performance harness until later refinement.
Record both as deferred, not passed; do not delay the first APK for an
accessibility-service setup or benchmark build. Existing semantics, touch
targets, large-font/IME support and privacy protections stay intact. Screenshot
review makes no frame-timing, memory or physical-device performance claim.

Ship this usable first slice without waiting for later voice, native
command/steer/queue, autocomplete or attachment work. Each later phase requires
its prerequisite protocol, security, storage and verified provider capabilities
and feature-specific proof before being advertised as supported. Current prompt
controls remain text `submit_or_steer`; do not invent distinct native actions or
enable unsupported delivery controls. The owner's explicit exhaustive-retention
deferral remains valid with its existing deferred evidence status and all other
gates intact. The later development-feedback approval defers this checklist as
a handoff prerequisite; it does not mark any item complete.

## Current infrastructure checks

One coordinator runs these after integration, not each worker mid-flight.
They check the current Rust sources/locks and shared development inputs, not the entire intended product acceptance suite:

```sh
nix develop path:. -c make check
nix eval --raw path:.#devShells.x86_64-linux.android.drvPath
nix eval --raw path:.#devShells.x86_64-linux.emulator.drvPath
```

`path:.` includes newly created files before Git tracks them; Git-backed
`nix develop` requires the relevant inputs to be tracked. `make check` runs
Nix/Rust formatting, tool-version smoke, locked Cargo tests, Clippy with
warnings denied, and both Cargo-lock audits against the pinned database.
`make format` modifies `flake.nix` and companion Rust. Database coverage and
supplementary advisory exclusions are explicit in the native source manifest;
an audit pass alone does not prove every required exclusion.
Configured CI additionally builds `goatr`, the Android native output and the
Kotlin-facing bindings/AAR. Local evidence and remotely executed CI evidence
must remain distinct. Version smoke proves tool executability, not a product.
The two `nix eval` commands resolve optional shell derivations without realizing
their closures; they prove neither an APK nor an emulator run. SDK license
acceptance remains project-scoped and explicitly authorized.

Actual isolated component storage smoke uses the same companion package:

```sh
nix develop path:. -c cargo run --locked --manifest-path companion/Cargo.toml --example state_smoke
```

It opens exclusively created private system-SQLite state and exercises
pre-write crash/reopen, uncertain receipts, matching lookup without replay and
revoked metadata denial. It does not authenticate a network peer or prove
provider dispatch/revocation. The existing full Cargo and installed Android
SessionStoreTest suites own source-epoch/page overlap, shared content, receipt
partial ordering, retained action discovery and finite cache/recovery bounds.

Optional Android tool execution, when the coordinator chooses to realize its
large closure:

```sh
nix develop path:.#android -c bash -c 'java -version && gradle --version && adb version'
```

## Implemented checkpoint commands and final proposed owners

The strict retention-crossing gate is implemented in the runner but remains
unexercised. Only a new isolated, recorded both-provider Android run can mark it
passed; component checks, source inspection and the bounded diagnostic do not.

The explicit diagnostic-only selector `--omp-creation-diagnostic --backend omp
--consent-owned-providers` uses the existing private creation checkpoint inputs
(`--config`, `--workspace-root`, `--goatr`, expected binary/lock hashes and fresh
`--evidence-dir`). It requires `GOATR_OWNED_OMP_BACKEND_CAPTURE=1` and the
validated owned-run relay seam. One fresh creation settles its original receipt,
then stops before prompts, Stop/history and Android. The shared reporter emits
at most eight closed sanitized guard lines; it does not add native probes or
relax admission. Failed resources/actions are not reused. Exact executed
commands and actual outcomes belong in [status](STATUS.md); this selector is
not a provider or Android acceptance pass.

`--codex-invalidation-diagnostic --backend codex --consent-owned-providers
--automate-native-inputs` uses the same private inputs for one fresh creation and
one independently witnessed native history command on its original binding.
Its validated owned relay enables the shared eight-line sanitized guard capture.
Existing shell-query enqueue/result/final-reproof failures report through the
same prebinding latch before projection, including the result-loop authorization
error. An enqueue acknowledgment is not result execution or delivery proof.
First reader/output/supervisor close causes use a separate latch in that same
budget and only the original authenticated Codex creation connection. They retain
closed families, optional actual error codes and bounded I/O/frame metadata before
teardown; missing codes stay absent. This includes the selected supervisor winner
before stop. Provider observation adds no native I/O, probes or deadline changes.
This branch returns before MCP dialogs, so it skips MCP registration/shims and
uses the configured approved outer wrapper with its unchanged app-server tail.
Ordinary MCP acceptance remains unchanged. Diagnostic success is not Android,
MCP, paging or foreground acceptance; failed resources are never adopted or replayed.

Selected Codex diagnostics preserve their original operation error while using
the existing 15-second cleanup budget to stop the original companion and finish
its stderr reader. Join occurs only after thread completion. The finite
`capture_drain` summary requires EOF and successful sanitized output delivery;
timeout, panic, read/output failure or rejected capture marks it incomplete.
Incomplete zero counters do not prove absence of emitted lines. Cleanup records
SIGTERM attempt/success separately and suppresses duplicate Drop signaling. A
signal labeled `after_helper_failure` is cleanup, not the initiating cause.
OMP and Hello behavior and provider deadlines remain unchanged.

The drain's finite shared snapshot survives an unfinished or panicked worker.
Its counters describe observations at summary emission, never future output.
Closed `termination` distinguishes `reading`, `eof`, `read_error` and
`thread_panic`; optional `readError` retains only I/O kind and bounded errno.
Separate `cleanupResult` distinguishes `stop_failure`, `drain_deadline` and
`joined`. Neither joining nor cleanup completion invents EOF. Snapshot locks
cover only finite field access, never pipe/stdout I/O, joining or awaiting.
The existing primary operation error and cleanup budget remain unchanged.

For pre-Ready relay failures, the selected helper retains the original settled
action and cutoff before subsequent fallible work. After stderr join and the
existing terminal observation, it may read that action's retained Relay evidence
and exact user-unit journal once (100 records/1MiB), using only remaining cleanup
time. Require original binding/root/boot/source and trusted service executable,
command, PID/invocation and timestamp joins. Exclude post-settlement teardown.
Only closed startup reports enter the existing shared eight-observation budget;
manager lifecycle records remain separate and never imply native readiness.
Missing/late/unjoined/failed capture stays unavailable independently of pipe EOF.
No new native RPC, process proof, replay, retry or deadline is authorized by this
read. Diagnostic-only journal routing and raw-error suppression must be exercised
before drawing a first-cause conclusion.

History observation errors retain their original read Method and static phase;
the enclosing command deadline is reported separately.

`native_guard_context_preserves_inner_phase_and_discards_private_errors` checks
the helper's finite private-path phase/category context, preservation of the
innermost phase and I/O kind, and omission of raw errors/paths. It does not prove
which path failed in an earlier unclassified run or repair its cause.

The named checkpoint owners use the existing Rust/Tokio and Kotlin/JUnit/Compose tools, with no additional runner package/framework. `companion/examples/smoke.rs` and `check_native_libs.rs` share the retained companion graph and are not installed product commands. Writable setup, both selected stock-provider modes, real installed UI, owned resource preparation, real IME and sealed native-camera producer now have source consumers. Command availability alone is not acceptance; actual scoped evidence is recorded in the [status](STATUS.md). Combined both-provider foreground attempts have not completed or been accepted; the disposable-setup gate remains unexecuted, and cleaned-camera revalidation is required. Export `GOATR_BIN` from the newly built absolute package output (never an older artifact), `GOATR_CONFIG` as the actual private deployment/template config, and `WORKSPACE_ROOT` as its canonical verified wrapper-accessible workspace. While inputs remain untracked, use `path:.`; do not rewrite locks at verification.

Set `EMULATOR_OWNER` to the current exact owned lease listed in [status](STATUS.md),
and recheck its original process before running. A serial alone is not authority.
For a narrow development continuity check, freeze the selected package hash first,
then run a fresh owned OMP/Codex setup. It uses direct public Herdr workspace and
agent launch for OMP, the existing fresh Codex creation path, a bounded native
history view, a real phone prompt and guarded desktop continuation, then a
phone reconnect, then a paired owned debug APK foreground consumer whose endpoint
and companion remain available for owner feedback. Paging is recorded as `proven` only when an explicit successor
cursor repeats stably and returns native provenance outside the current live
window; otherwise it is recorded `unproven` after at most 64 native commands.
It does not run the final Android foreground acceptance, Stop/dialog/recovery
gates, or cross the native retention boundary.

When the development runner reports `ready`, it keeps the exact fresh companion,
native sessions and paired debug app alive until Ctrl-C or SIGTERM. Its emitted
`desktopAttach` object supplies the nonsecret same-XDG command for the retained
Herdr session: run its executable with `session attach <name>` and exactly the
listed `HERDR_STARTUP_CWD`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME` values. The
runner rechecks that command's original Herdr server/socket before publishing it.

```sh
GOATR_SHA256="$(sha256sum "$GOATR_BIN" | cut -d' ' -f1)"
GOATR_LOCK_SHA256="$(sha256sum companion/Cargo.lock | cut -d' ' -f1)"
GOATR_APK_SHA256="$(sha256sum "$GOATR_APK" | cut -d' ' -f1)"
GOATR_TEST_APK_SHA256="$(sha256sum "$GOATR_TEST_APK" | cut -d' ' -f1)"
nix develop path:. -c cargo run --locked -j2 --manifest-path companion/Cargo.toml --example smoke -- \
  --development-continuity --backend omp --backend codex --goatr "$GOATR_BIN" \
  --expected-goatr-sha256 "$GOATR_SHA256" --expected-companion-lock-sha256 "$GOATR_LOCK_SHA256" \
  --expected-apk-sha256 "$GOATR_APK_SHA256" --expected-test-apk-sha256 "$GOATR_TEST_APK_SHA256" \
  --config "$GOATR_CONFIG" --apk "$GOATR_APK" --test-apk "$GOATR_TEST_APK" \
  --emulator-owner "$EMULATOR_OWNER" \
  --workspace-root "$WORKSPACE_ROOT" --consent-owned-providers \
  --automate-native-inputs --evidence-dir evidence/development-continuity
```

For development feedback, append `--development-android-only` to the
`--development-continuity` command above. This explicit mode preserves fresh
owned OMP/Codex bindings and original identity/permission checks. Both providers
use regular receipt-fenced SessionCreate and independent native proof; this mode
selects that path before launch and never falls back after failure. It omits the Rust
history-generation/continuity prelude, and selects
`GoatrConnectionTest#development_feedback_android_to_desktop`. It records one
Android history attempt per provider as `observed` or `unproven`; neither label
proves stable older-page cursors. After that attempt, the consumer must return
to current history and prove the same original Target Ready before Send.
Authoritative receipts for the exact action, actual native tool effects, desktop
continuation, foreground reconnect, prior-action reconciliation and once-only
effects remain mandatory. The feedback consumer defers canonical receipt-details
scrolling as a known UI issue and never fabricates an outcomeShown acknowledgement.
Its retained evidence marks details reachability unproven. The ordinary continuity
consumer and full acceptance keep the canonical display check.

A complete history record does not establish terminal tool success. Both consumers
wait for explicit Succeeded status with one keyed native call identity, retaining
every marker-matching record. Running/Unknown may wait within the existing
deadline; Failed, missing identity or multiple calls reject immediately. A
Dispatched prompt receipt is not a native tool result.

Development Android-only and owner-feedback creation pin the successful receipt
Target including generation, native ID, binding and native routes before proof.
They select that exact manifest once and may observe only its presence until the
existing first-scope deadline. Physically validated Past metadata can pin the
original connection/relay/TUI for comparison; only fresh Connected presence with
the original associated/resumed thread grants authority. State/membership loss,
identity changes, future timestamps and malformed records fail immediately.
The Android-only initial Codex proof has one absolute 60-second read-only
observation budget. Owner-feedback retains 30 seconds. Fresh presence, original
receipt/binding/source/process pins and immediate identity rejection remain
required. Final complete scope checks for both providers run immediately before
Android handoff with their existing scope budgets. The earlier duplicate Codex check is omitted;
input-boundary checks and full-mode ordering remain unchanged.

The temporary WAKE controller keeps its authorized capability. Linux Desktop
children clear ambient, inheritable, permitted and effective sets before exec;
the independent internal Codex relay clears them before runtime worker creation.
A fresh native run must verify all four sets are zero in the original Herdr,
provider owners, TUI and relay, including their observed threads. Controller
capabilities, service properties and global policy are unchanged.

Automation reveals virtualized controls before asserting display and enabled
state, then rechecks the original exact-Target readiness before accepting the
source. The readiness wait has one monotonic 30-second budget, including scroll
work; it cannot accept late success or reset the budget.

The development consumer observes completion of its single Refresh click under
one separate absolute120-second budget, including the click and wait. It still
requires the original Target, a changed non-null subscription, current history
and completed operation, then performs the existing readiness check. It never
repeats the click or accepts late success. A failure captures only the last
closed predicate sample (unobserved before polling), existing notice category
and finite failure family. Product/provider/request/frame deadlines are unchanged.

The helper’s completed-tool observation has one absolute 180-second budget,
including the complete read-only open and inter-poll waits. Expired observation
starts no new read and cannot accept late success. Instrumentation remains
polled during control authentication, native handling and response writes.
Control responses and owned ADB-child completion/teardown have bounded waits;
teardown preserves the primary failure. Cancellation never retries input.

The current new-action call may wait for existing read-only recovery before its
first frame within its original request deadline. Acceptance still requires the
original Live/boot/Target/caller, one pre-send record, at most one mutation frame,
and the authoritative canonical receipt. Replacement, cancellation and expired
admission remain closed. Legacy review-fixture tests are separate from the
actual two-provider Android continuity consumer and are never a live pass.

The consumer waits for the actual Send control to be enabled with the exact
selection/reservation and current history, then clicks once. Development-only
instrumentation reports at most64 closed phase/provider/state snapshots; the
helper retains the active failure phase, receipt authority and a separate
cleanup-failure flag. Unknown/private values are discarded. Cleanup preserves
the original failure. These diagnostics add no provider reads, action retries
or acceptance claims.
Once-only tool evidence compares the original native execution `callId` across
reconnect, requiring one successful keyed execution. Multiple live/persisted
display rows for that same call do not imply another execution; empty, unkeyed,
different or multiple calls fail. Retained instrumentation evidence contains
only closed row/call counts and identity-match state, never the call IDs.

For a known-failure owner handoff, use `--development-owner-feedback` instead
of `--development-android-only`; the flags conflict. This mode validates the
artifacts and owned emulator, creates and proves fresh original native bindings,
and retains that exact companion/listener/native context. It installs no APK,
sends no Android input and makes no Android acceptance claim. Its exclusive
`development-owner-feedback-RUN.json` records `ready_for_owner`, known failures,
and exact nonsecret local pairing/desktop-attach commands. Pairing generates a
fresh invitation only when the human runs the printed command in a terminal.
Readiness remains valid only while the original helper and owners remain alive;
failed resources are never adopted. Stopping the helper stops its companion and
preserves native work. Human phone confirmation remains pending.
Only this explicit owner-feedback mode may retain a presentation mismatch as a
known failure: record focus change and nullable geometry change, with no no-focus
acceptance claim. Preserve short-circuiting, successful/error-free receipt,
same-action lookup, exact native proof, capability and readiness checks.
Observation errors and uncertain creation remain fatal; never restore focus or
resize. All acceptance paths retain the fatal presentation comparison.

```sh
nix build path:.#goatr path:.#iroh-android
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --all-targets
nix develop path:. -c cargo fmt --manifest-path companion/Cargo.toml -- --check
nix develop path:. -c cargo clippy --locked --manifest-path companion/Cargo.toml --all-targets -- -D warnings
nix develop path:.#android -c bash ./gradlew :iroh-jvm:test :app:testDebugUnitTest :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug --dependency-verification strict --stacktrace
nix develop path:.#android -c cargo run --locked --manifest-path companion/Cargo.toml --example check_native_libs -- app/build/outputs/apk/debug/app-debug.apk
nix develop path:. -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --transport direct --goatr "$GOATR_BIN" --evidence-dir evidence/direct
nix develop path:. -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --transport relay --goatr "$GOATR_BIN" --evidence-dir evidence/relay
nix develop path:. -c cargo run --locked -j2 --manifest-path companion/Cargo.toml --example smoke -- --backend omp --backend codex --goatr "$GOATR_BIN" --config "$GOATR_CONFIG" --workspace-root "$WORKSPACE_ROOT" --consent-owned-providers --automate-native-inputs --evidence-dir evidence/providers
```

Future plugin lifecycle execution uses the **same** smoke example, not a separate runner/package. Run in the pre-provisioned disposable Linux test environment described below; `$release_rev`/`$upgrade_rev` must be separately reviewed real immutable public commits containing the implemented plugin/Rust package. No supplied commit is implied and this command is unexecutable until those artifacts exist:

```sh
nix develop -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --herdr-plugin --release-ref "$release_rev" --upgrade-ref "$upgrade_rev" --evidence-dir evidence/herdr-plugin
```


The owner explicitly approved one fresh private API34/default/x86_64 feedback
AVD after the original ce8ae66d resource repeatedly showed OS ANRs. Its exact
run is `48b82c85-dd6c-4a93-9bd6-051456029000`, root
`/home/trev/Workspace/.gs-feedback-48b82c85-dd6c-4a93-9bd6-051456029000`,
AVD `goatr-feedback-48b82c85`, serial `emulator-5582`, two cores/2048MiB,
using the already pinned SDK image. Preserve the original AVD, leases and
evidence. The selected live helper consumes only the new exact owned lease;
arbitrary serial adoption, global SDK/settings changes and physical devices
remain unauthorized. Initial creation records actual original process birth
before publishing its private lease. Recovery preparation still refuses a live
selected serial/PID or reused/unowned process and restarts only that recorded
stopped AVD. Ordinary UI/components independently verify the same lease.
Historical fixture procedures below retain their explicitly named original
resources; they are not authority to adopt either resource by serial.

```sh
nix develop path:.#emulator -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --prepare-emulator --api 34 --serial emulator-5582 --emulator-owner "$EMULATOR_OWNER" --evidence-dir "$EVIDENCE_DIR/prepare"
# Standalone real visible keyboard keys, no stock-provider inference:
nix develop path:.#android -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --ime --serial emulator-5582 --emulator-owner "$EMULATOR_OWNER" --evidence-dir "$EVIDENCE_DIR/ime"
# Standalone actual CameraX frames/ZXing/inverted never-issued QR + Back/reentry:
nix develop path:.#android -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --camera --serial emulator-5582 --emulator-owner "$EMULATOR_OWNER" --evidence-dir "$EVIDENCE_DIR/camera"
nix develop path:.#android -c cargo run --locked -j2 --manifest-path companion/Cargo.toml --example smoke -- --ui --serial emulator-5582 --emulator-owner "$EMULATOR_OWNER" --backend omp --backend codex --goatr "$GOATR_BIN" --config "$GOATR_CONFIG" --workspace-root "$WORKSPACE_ROOT" --consent-owned-providers --automate-native-inputs --evidence-dir "$EVIDENCE_DIR/stock-ui"
# Final-only background owners below remain proposed/unimplemented, not checkpoint passes:
# --background persistent|ntfy requires its separate final implementation/authority.
```

`--ui` installs the actual supplied debug/test APKs, drives production MainActivity through Compose semantics and protected actual native owner, receives live pairing only in bounded owned memory-only loopback/reverse control, and correlates real native history/tools/prompt/Stop/dialog/create/recovery/revoke effects with exact kernel/Herdr/owner/relay/TUI identities. Fresh nonsecret PNGs are taken only after raw pairing/secret/URL scenes are gone and FLAG_SECURE is false; raw hierarchy/logcat/originals are never retained. Native camera/IME helpers retain honest component labels, never stock/provider passes. Final background/distributor/notification-entry and API36/real16KiB resource owners remain separate unexecuted final requirements; no other AVD is provisioned by this checkpoint runner.

Fresh UI/component installation resets only `io.github.trevarj.goatr.debug.test`
then `io.github.trevarj.goatr.debug` on the independently verified owned emulator.
It refuses a live prior app or ambiguous package presence, uninstalls each
present package once, verifies absence, then installs the supplied APKs once.
A cached test-only process is covered by this package reset; it is not adopted
as native identity. Separately protected active instrumentation remains untouched.
It never changes keys/signatures, other packages, physical devices or global
settings. A failed/uncertain effect stops the runner without retry. Installation
failures retain only a finite known package-manager category and closed
app/instrumentation stage, never raw diagnostics. Exact installed APK identity
is independent evidence; an installation pass is not foreground acceptance.
Development readiness timeouts emit one closed snapshot of the final existing
predicate values before cleanup, retaining the original exception and deadline.
The matching helper parser rejects unknown fields/values; this diagnostic adds
no native requests, retries or readiness authority.

The user-authorized `--defer-retention-crossing` helper option requires `--ui`,
both `--backend omp --backend codex`, `--consent-owned-providers` and
`--automate-native-inputs`. It defers only exhaustive1000-record generation,
outside-retention native paging/repeat proof and the corresponding Android
older-witness/taps/return-to-latest assertions. Ordinary actual native history,
tools, prompts, dialogs, modes, Stop, receipts, identity, recovery/process death,
transport and revocation gates remain required. The authenticated owned UI
control supplies the single deferral boolean; no older IDs are fabricated.
Evidence records `exhaustivePaging:"deferred_by_user"`, `olderPagingProven:false`
for each deferred provider gate, and terminal status
`foreground_checkpoint_paging_deferred`. This is an accepted foreground
checkpoint with paging evidence deferred, not a full paging acceptance pass.
Without the flag, the strict retention gate and default behavior remain required.

For the requested usable foreground feedback checkpoint, append
`--keep-foreground-for-feedback` to that full `--ui --backend omp --backend codex`
command, with both existing provider-consent flags. This non-installed helper
option rejects component and single-provider modes and cannot resume a prior run.
Every selected acceptance gate and its cleanup still runs, including actual OS death,
Android revoke/Forget, runner revocation/hello rejection and companion stop.
Only after `stock-RUN.json` records PASS or the explicit
`foreground_checkpoint_paging_deferred` result does the same helper restart its
scoped companion with the original still-owned OMP listener descriptor. Feedback
ready/outcome evidence preserves the paging deferral label.

A separate signed instrumentation consumer requires an empty installation,
freshly pairs through visible UI with the existing memory-only control channel,
checks both original native Targets and actual installed APK hashes, then
shuts down its transport while retaining that pairing. The helper removes its
exact reverse mapping and launches normal MainActivity. Exclusive
`feedback-ready-RUN.json` distinguishes the authenticated instrumentation
observations from the subsequent production launcher/process observation;
it is not another provider acceptance pass. The helper must remain running:
it supervises original emulator/desktop/provider/companion/app process identities
until interrupted. Interruption or failure stops only its matching app/companion
and revokes its new phone peer; desktop work remains retained.
`feedback-outcome-RUN.json` records that end separately from acceptance.
No pairing token, QR, clipboard input, screenshot, raw hierarchy or native
payload is retained by this continuation. Source availability is not evidence
that this post-pass path has run; coordinator validation and the fresh full
both-provider Android run remain required.

The test-only `OwnedInstrumentationRunner` emits the original kernel PID/start
for every invocation before tests begin. Stock runs additionally generate an
in-memory Java platform EC `secp256r1` signing key; only its PUBLIC uncompressed
65-byte identity accompanies that exact owned `am instrument` status stream.
The parent joins the target PID/start before control admission. No private key,
bearer fixture or pairing URI is provisioned through arguments, environment,
files, clipboard, logcat, screenshots or test output; `stockControlPort` is only
a routing address, never authority. Each connection receives a fresh random
32-byte challenge. The request carries public65, one-byte DER-signature length
(8..72), signature, big-endian u32 request length (1..65536), and those exact
closed JSON bytes. Platform `SHA256withECDSA` signs
`GOATR-STOCK-CONTROL-v1\0 || challenge || length || bytes`; retained ring0.17.14
verifies P-256/SHA256/ASN1 before JSON parsing or pairing/admin/process/capture
effects. Unknown keys, unsigned, malformed, tampered and replayed requests close
without an application response. A recovery key/epoch is admitted explicitly
only after the original proved OS death and old owned stream exit, never from
a caller-supplied key or a late process sample.
Method-only controls use empty struct variants under the same Serde
`deny_unknown_fields` schema, not internally tagged unit variants that discard
extra members. The authenticated TCP regression checks admission of all five
exact method-only shapes and a payload variant, rejection of extra members on
each, and literal/escaped duplicate-key rejection by the unchanged raw native
JSON parser. Diagnostic testcase labels are static, never bodies.


Cancellation encloses the complete controller operation, including real desktop
input and long handlers. Exact original-process cleanup and this run's unchanged
reverse mapping are attempted afterward; replacements/unproved processes remain
untouched and cleanup is refused. Cancellation retains potentially committed
native work and original receipts, without replay, rollback or a no-effect claim.
The component wrapper uses the same original-process rendezvous and cancellation;
camera release/policy/pose cleanup still has its separate native observations.
Coordinator regressions (unexecuted for the corrected source):

```sh
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --lib setup::tests
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --example smoke private_control_rejects_foreign_unsigned_unknown_key_tamper_replay_and_unclosed_bytes
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --example smoke teardown_and_death_never_adopt_a_replacement_or_unproved_app_epoch
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --example smoke signal_cancels_actual_terminal_input_barrier
```

These exercise actual disk-source, cryptographic TCP admission and real private
PTY-input cancellation boundaries, not a live provider/disposable-manager/device
pass. The actual corrected APK/`--ui`
gate must additionally reject a foreign local caller before `live_pair` can issue
a capability, survive the intentional original-process death into fresh signed
recovery, and be cancelled with SIGTERM/Ctrl-C while awaiting the real `Desktop`
or `TriggerDialog` input barrier. Observe exact cleanup and retained native work.
For teardown/death replacement tests, independently replace the recorded target
before cleanup: the runner must refuse and leave that replacement alive.

Runner output is machine-readable gate name/pass/fail/blocked, actual command, versions/API/ABI, sanitized identifiers/timing and artifact paths. Nonzero on failure; a blocked prerequisite is separately recorded and cannot be counted as success. Runtime runner must not silently skip a selected backend. Unit fixtures may use deterministic provider transcripts/fault injection; live gates require actual providers.

### Exact source-owned checkpoint scenarios

Both provider flags are mandatory for checkpoint completion. A selected backend
never silently skips: absent installed wrappers/source views/credentials/shared
mode/user manager/native forms/history continuations/zero-turn resume produce
blocked nonzero evidence. Each root is exclusive private `.gs-*`, with unchanged
real runtime, exact owned admin override on every direct CLI call, isolated named
Herdr/project/owner sockets/units and an already-bound OMP loopback listener passed
by validated kernel FD/source record. Production port7466/endpoints are untouched.
The same ordinary packaged serve/transport/native creation paths are used; no
transport-smoke/provider fixture bypass or copied credential/signing workaround.
`--automate-native-inputs` adds explicit per-invocation consent for real inputs
to this fresh run's exclusively owned native TUIs. Without that flag, existing
manual operator phases remain available. Automation never enters a synthetic
`yes`, substitutes a Goatr prompt for independent desktop input, or falls back
to a different pane/session/engine. Acknowledgement alone never passes an
observation. The consumer verifies actual native pages/tools/markers/Stop,
both-side dialog closure and late no-effect replies, focus, matching receipts,
connection gap/restart, PID/native identity preservation, partial crash resources
and matching-old-action status without another workspace/thread/TUI/native write.
Created desktop work is recorded and retained, never rolled back or wildcard-killed.

#### Owned native input automation

The existing smoke example uses `herdr::Client` on the exact owned named socket.
Before each input and afterward it re-proves the original Herdr process/socket,
public pane/internal terminal/workspace, provider owner/TUI/relay processes,
Target, native ID and native connection/room epoch. `agent.prompt` sends one
ordered native text+Enter command; `agent.send_keys` sends one guarded key queue.
The public pane ID comes from the actual current response, never focus or a
derived terminal name. Installed Herdr checks the actual foreground agent, but
offers neither exact-PID compare-and-enqueue nor recall. Changed/unknown outcomes
retain native work and metadata-only input intent; there is no retry or redirect.
No prompt/key payload, raw TUI, room URL/key or native error message is retained.
Marker prompts explicitly request the exact echo without files/other sessions;
independent same-Target history remains the authority.

Native input and mode-restoration evidence uses an immutable exclusive intent
`native-{input|mode}-RUN-ID.json` and at most one separately exclusive terminal
`native-{input|mode}-RUN-ID.outcome.json`. Both retain the same run/action ID
and source metadata; outcomes never replace intents or adopt pre-existing files.
Count acknowledged/restored terminal states once by action ID, never both files.
An intent without a terminal record remains unconfirmed and never authorizes
input replay; even acknowledged input still requires independent native effect
observation. The real filesystem regression
`native_input_and_mode_outcomes_preserve_exclusive_intents` checks coexistence,
unchanged intent objects/bytes, private permissions and refusal to replace known
or unknown terminal records.

OMP uses the installed `ask` tool. For the UI sequence, the actual empty-option
select must expose exactly its native Other/Chat choices; Android chooses Other
and the next gate observes the distinct real editor request. Non-UI editor
preparation answers only that actual preparatory select through Goatr.
Codex approval requests a harmless native `exec_command` printf with
`sandbox_permissions=require_escalated`, without changing permission defaults.
Questions use the native `/plan` command, actual question ID/options/blocking
proof, and the relay's original correlated mode observation. Restoration is only
mode-kind restoration: after actual closure/idle, at most one native BackTab is
sent when the actual Default/Plan catalogue proves it, followed by a same-source
settings-update observation. No full model/effort/developer-instruction restoration
is claimed and missing mode evidence never becomes an inferred Default.

Form/URL inputs require a real scoped MCP counterpart. Before a new owned owner
is created, the helper generates a private test wrapper which executes the
original approved outer Codex wrapper with MCP `-c` prefixes **before** the
unchanged app-server/listen/socket tail. TUI and other arguments pass unchanged;
the native inner binary, OS sandbox, signing and default permission policy remain
unchanged. Only this invocation's configured wrapper and candidate Herdr PATH
change; the actual production shell probe/creation proofs must confirm resolution.
No global trust/config, credentials, installed unit, account or extra engine is
changed. The same compiled smoke ELF is copied into the protected owned project,
with source/copy hashes and ownership recorded; no new helper package is installed.

Its internal `--owned-mcp-stdio` mode implements actual MCP initialize,
tools-list/call and `elicitation/create` response correlation for two nonsecret
label tools. It checks its exact image/lease/native parent and claims each owned
operation once. The URL tool serves a real bounded loopback label form in the
recorded desktop network namespace; GET is not completion. Actual decline/cancel
is durably recorded without label success. The UI gate requires the real service
to receive the native decline after the source-bound card closes. Native managed
allowlists, Never policy, missing capabilities or namespace access may honestly
block; automation does not weaken them or fabricate cards.

The private service publishes verified-startup metadata only after its unchanged
lease, full executable digest and process checks, plus original-pipe validation.
The stdio regression waits for that exact child's run/server/image/CWD/process
record within120 seconds, **then** starts each unchanged15-second real response
deadline. This is not an MCP initialize success and does not skip the digest.
Actual native Codex sends initialize on spawn; its pinned
[RMCP startup default is30 seconds](https://github.com/openai/codex/blob/8e68a98ef03cdde76d2e6800791ebdf1b3b95b24/codex-rs/codex-mcp/src/rmcp_client.rs).
The generated wrapper currently overrides only this server's command/args, not
its startup timeout. A measured `verifiedStartupElapsedMs` must inform any
explicit owned-process startup-budget decision; passing the staged regression
does not prove compatibility with the native30-second handshake budget.
Historical opaque receive timeout alone does not establish digest timing.
Input uses a separately opened nonblocking original pipe with cancelable
AsyncFd reads; inherited stdin flags and the service120-second elicitation
deadline are unchanged. An error-bearing/ambiguous response cannot apply a
form label or become successful decline evidence.

Older-history generation uses sequential native `!printf` commands only while
idle, with unique markers and observed completion before each next input. OMP
keeps one independently authenticated guest from its original writable welcome
and FINAL, continuously draining bounded frames with one registered command
expectation. Registration precedes dispatch; only the exact unique live
bashExecution ID/command/marker line, exit0, not-cancelled/not-truncated entry
can complete it. That same completed ID must reach the original Goatr Target
Ready/Idle without pending requests. Duplicate completion, stream/source loss,
malformed/authentication failure or timeout permanently fails that observer;
there is no rejoin or replay. The helper retains the first observed closed failure
under its existing mutex, so later reader/scope invalidation cannot overwrite it.
The existing error contains only a static phase and applicable allowlisted I/O,
WebSocket or native-frame family. No payloads, raw errors, IDs or paths are retained;
these fields do not establish the remote initiating cause or add native reads.
Codex completion still comes from the actual
completed command tool record. The separate phone/model/native-tool gate is
unchanged. Fresh before/after input scope checks retain registry, native owner,
Herdr process/socket/pane/terminal, protected history object/header, Ready,
focus and geometry guards. The captured OMP history witness scans only appended
bytes and permits only its original legitimate lazy materialization.
Codex strict generation uses one original session.open subscription and one
independent read-only native witness on the **same original owner/socket/thread**.
The witness may initialize, check complete loaded membership, read exact thread
metadata and list the newest flat item page. It never resumes/starts/forks a
thread, invokes shellCommand, changes settings or answers requests. Native IDs
and turn IDs are compared literally, without Goatr's projector. Any failed read,
source change or uncertain input terminates the attempt; no replay or fallback.
Each command requires the exact native completed commandExecution, original
literal shell argv/script, unique marker, completedAtMs, completed status, exit0,
no truncation, plus Goatr completion/Ready/Idle/no-pending on the original stream.
ACKs and incidental native records never increase the confirmed command set C.

The separate noninstalled `--paging-proof` mode is `BoundedDiagnostic128`:
Codex-only, zero-item, 128 guarded commands, foregroundAcceptance=false and
`strictRetentionBoundary=false`. Its 60-second outer envelope and 30-second
input/completion limits do not prove a retention crossing.

The normal non-deferred full foreground run is `RetentionBoundary1024` for both
providers. It creates 1,024 independently completed native commands plus one
guarded live append, under a 60-minute phase ceiling, 30-second per-command
limit and the existing 16-command throughput qualification. OMP records the
ordered independently consumed observer ledger; Codex records literal native
ID/turn pairs from its read-only same-owner witness. After an ordered `phone.host`
barrier, the original Ready/Idle/no-pending generation view freezes completed
consumer set R with |R| <= 1,000 before any browse request.

The strict result requires a completed native ID absent from R with at least
1,000 later independently confirmed completions. It retrieves every generated
ID through the original native view, with at most 64 cursor advances and its
15-minute expiry. It captures an explicit cursor page containing that exact
witness only after provenance and creation-order validation, repeats the same
cursor with a fresh request ID and identical records/order/display/successor,
then sends exactly one append and repeats that captured cursor again. OMP rows
must be Persisted; Codex rows must be NativePage with the exact native turn.
No cache, alternate engine, cursor synthesis, retry or history-wide rescan is
accepted.

The Android consumer receives the exact qualified ID, source and later-completed
lower bound. It reaches that ID only through real browse/load-older controls,
keeps the visible window at 1,000 rows or fewer, displays the exact row, returns
to latest, and only then performs later mutations. A real run must record the
strict result for OMP and Codex before this gate is accepted. Empty pages, ACKs,
source fixtures and the bounded diagnostic establish none of this proof.

Uncached Codex advances perform one native history-page RPC with fresh physical,
complete loaded-before/read/loaded-after/physical guards, a14s operation budget,
64 total RPCs and16MiB aggregate response bytes. Pages/cursors are immutable after
capture, coalesced per ticket, and charged to shared64MiB/64-ticket accounting.
No local-cache pagination, retained-boundary seeking, unbounded content hydration,
lexicographic native cursor comparison or extra diagnostic reads supplies proof.
Source/fixture/test passes remain separate from actual native acceptance.

Android receives only bounded independently proved older IDs and original
Target/provider/native-owner provenance from that gate. After the existing
explicit companion recovery, its controller independently re-proves the current
owned Target and those exact IDs beyond the current latest snapshot. This is
bounded backscroll coverage, never a claim of unlimited provider retention. The
stock test taps the real older-history
control at most64 times on one unchanged Target/subscription, checks cursor
progress, scrolls the exact witness row into view and asserts it displayed.
It then uses the real Return to latest history control before continuing prompt,
tool, dialog and Stop gates. Older browsing uses the same bounded Android display
window, preserving requested older rows across live updates while retaining the
server cursor; a fresh installed
snapshot returns that window to latest.

Coordinator checks after the complete source freeze remain limited to at most
two build/test workers. Relevant regressions:
`omp_observer_` in the smoke example covers one initial FINAL with32 registered
live completions, immediate-completion races, duplicates/invalid results,
initial scope/queue bounds and actual framed authentication/disconnect failure.
It also checks first-failure retention, closed family distinctions and payload privacy.
`periodic_registry_deadline_is_typed_and_distinct_from_malformed_source`,
`native_handoff_pins_first_boundary_per_peer_view_and_expires_old_source`,
`global_history_prefix_eviction_clears_native_boundary_and_rebases_without_gaps`
and `continuation_eviction_never_aliases_original_boundary_to_successor` cover
the corresponding bounded native/state seams. Android
`SessionStoreTest#olderWindowRetainsRequestedRowsAcrossLiveUpdatesAndResetsOnlyOnSnapshot`
covers capped older browsing, live-field precedence and stale-page rejection.
`history_snapshot_bytes_follow_` compares cached totals with actual serialized
records through live insert/upsert, pinned snapshots and record-limit trimming;
the global-prefix regression also checks native-page accounting and real budget
eviction. `private_read_retries_retired_publications_without_returning_old_bytes`
and `private_read_rejects_unsafe_files_and_replacements_without_retry` exercise
atomic publication replacement on ordinary private fixture files, the unchanged
three-attempt ceiling and terminal unsafe-file guards. These are component
regressions, not native paging or owner-admission acceptance.
`framed_mode_observation_is_correlated_private_and_connection_scoped` exercises
real Unix-WebSocket frames, typed correlation, late replies, epoch clearing and
private-field exclusion; `owned_mcp_stdio_decline_is_correlated_once_and_private`
drives the actual owned stdio service and HTTP GET/decline, including duplicate
operation rejection and an ambiguous accept+error producing no label or decline.
Its actual open-pipe reader/deadline boundary must return through normal Tokio
runtime teardown while the parent keeps stdin open; it does not fake120 seconds
of elapsed native time or bypass shutdown with `process::exit`.
These are regressions, not stock-native or Android passes.
Only the new real automated run may complete the remaining acceptance.


Previous scoped native runs reached phone/model/tool, desktop marker and
original-Target Stop/Ready/Idle gates; one also proved actual OMP older history.
Those helpers have ended. Retained cgroups or desktop children do not authorize
input or adoption. Current observations and exact artifacts belong only in
[Status](STATUS.md), not this acceptance contract. The coordinator owns the final
clean package and fresh combined OMP + Codex + Android/dialog/recovery run.
Scoped milestones never permit replay of uncertain actions or satisfy the
remaining gates.

The later temporary Owner-stage capture is a separate scoped diagnostic: only
the validated owned relay enables it, ordinary/OMP runs stay silent, and the
existing shared reporter permits at most eight closed lines. Capture the first
terminal returned guard before generic projection, preserving original errors,
I/O categories, reads, retries, authorization lock scope and deadlines. It is not
creation or Android acceptance proof and is removed after causal-fix exercise.
The same reporter may retain a dropped original Thread effect's closed phase,
preferring its existing pending reader failure. Cancellation alone proves
neither a timeout nor native failure. External helper projection may retain its
already-computed closed Codex association state/resume/error-family/method/code
fields; it adds no native reads or readiness authority.

The temporary original-invalidation capture was removed after the repaired first owned Codex `!printf` completed on its original Target and generation. The publisher-valid nullable `aggregatedOutput` start value is initialized only for the textual command-output event; other non-string cached slots remain fail-closed. That isolated run deliberately stopped before history paging or Android acceptance, so the final two-backend Android command remains mandatory.

`--ui` owns two exact consumers, not a whole `GoatrConnectionTest` class run:
`stock_packaged_host_actual_main_activity_both_selected_providers` and
`stock_process_death_restores_original_target_receipt_without_replay_then_forgets`.
The first reaches actual source-correlated UI/native operations and an observed
owned APK PID/start OS death after its real pre-send copy. Only that recorded
death permits the second to reuse its protected identity and inspect the exact
original Target/UUID/deadline/receipt without native replay, then exercise revoke,
cancel/confirm local Forget and actual deletion. An arbitrary pre-existing app/
identity is refused; recreation/fixture source is not OS-death or stock evidence.

`--camera` is the standalone producer/consumer command: source-fixed synthetic
never-issued inverted QR and neutral grayscale PNG exist only in sealed retained
memfds. It uses ONLY the existing private owned controller advertisement/kernel
loopback listener and sensitive memory-only auth header, fixed installed
PhysicalModel get/set protobuf and bounded HTTP2 DATA/trailers requiring final
grpc-status0. No endpoint/service is started, no inline setPoster image persists
bytes, and no raw capability enters argv/env/log/clipboard/file/screenshot.
Before building the native acceptance HTTP2 client, the runner installs the
already-selected rustls ring provider process-locally only when no default is
present; initialization refusal is a visible error, not a panic or suppressed
fallback. This changes no host settings, dependency choice or TLS verification.

Pose admission keeps the five-second readback deadline and original per-component
0.005 position-distance bound. Rotation is checked as the same physical
orientation within **0.005 degrees**, using f64 XYZ (`X*Y*Z`) unit quaternions
and their `q`/`-q` equivalence, not component-identical Euler triples. Actual SDK
readback normalized requested `[0,-150,0]` to `[180,-30.000004,180]`; this is
equivalent orientation, not a setter ACK or a camera pass. The same comparison
guards original-state restoration. Temporary vector telemetry is removed;
failure context names the calibrated bound, deadline and preparation/restoration
only. The pure regression covers native normalization, wraps, quaternion sign
equivalence and genuine rotation/position boundary rejection; actual CameraX/
ZXing and closed/restored observations remain the success authority.
The primary [PhysicalModel setter/getter](https://android.googlesource.com/platform/external/qemu/+/refs/heads/emu-master-dev/android/android-emu/android/physics/PhysicalModel.cpp)
and [XYZ/quaternion helpers](https://android.googlesource.com/platform/external/qemu/+/refs/heads/emu-master-dev/android/android-emu/android/physics/GlmHelpers.h)
explain that representation; source inspection is not an installed-source
byte-identity claim.

Native filename/setter ACK is never readiness: actual pose readback and actual
CameraX/ZXing preview/Back/closed/reentry assertions are required. Fixed
`fresh-start` PID/start and `fresh-closed` status joins establish this invocation's
APK ownership; cleanup refuses replacement/unproved actors, waits actual camera
release, resets the owned default wall and restores captured pose before dropping
FDs. Existing live/unowned APK processes block before installation. Unsupported
native cleanup/source/client evidence is blocked, never an invented pass.
The native camera test reports assertion failures only as a fixed named stage
and exception CLASS on its owned instrumentation stream, never exception messages,
state, raw stacks, semantics, capabilities or pixels. Fixed stages cover
before-scenario, first native open/QR preview, Back/reentry/paste/rapid Back,
policy/error/retry/fresh QR and final restoration. First-QR substages identify
synthetic identity/authority display/confirm presence/no-pair assertions; paste
substages identify privacy/empty edit/manual preview/authority/no-pair/close.
Parent capture bounds and validates at most the primary and final-restoration
records.
Failed gates retain only these records, reached fixed producer phases and
original-source/restored/closed/cleanup booleans. Phase ACKs cannot grant readiness.
Existing capture-unavailable and unsupported-error-trigger codes remain blocked.

Actual nonsecret view calibration found a clipped upper poster edge and foreground
table occlusion at `[-2.006,0.350,3.119]`. Raising ONLY Y to `1.0`, with X/Z and
`[0,-150,0]` rotation unchanged, produced a full checker/poster border above the
furniture. The inspected native
[PhysicalModel getTransform](https://android.googlesource.com/platform/external/qemu/+/refs/heads/emu-master-dev/android/android-emu/android/physics/PhysicalModel.cpp)
copies position X/Y/Z directly to translation, while the
[controller coordinate contract](https://android.googlesource.com/platform/tools/base/+/285ce4c92366c4b0b1152cc7ccd1f2d862320d7e/emulator/proto/emulator_controller.proto)
documents Y pointing up. The producer retains this evidence-derived height with
the original strict pose tolerance/deadline and neutral preload. The temporary
checker/screenshot/`FLAG_SECURE` override, independent frame analyzer, native PNG
decoder and Retry/route metadata probes are removed. Scanner/paste privacy remains
enforced throughout. The coordinator recorded the complete actual six-phase
CameraX/ZXing/native-policy-error/paste/Retry/distinct-fresh-QR/Back-closure pass,
with exact original source joins and pose/policy restoration. This isolated
API34/x86_64 component pass is NOT live camera authorization, physical hardware,
stock-provider feasibility, emulator source reproduction or real16KiB evidence.
Native filename/setter ACK and visible poster geometry still cannot grant success.

Standalone IME really clicks the visible AOSP minus/digits/dot keys to enter
-1.5 in[-2,-1] and integer-1. The native-camera consumer additionally runs two
bounded rapid Back/reentry cycles, then captures actual guest-user/software camera
sensor privacy through the native framework, validates the installed shell
`enable USER_ID SENSOR`/`disable USER_ID SENSOR` API, induces policy while bound,
and observes actual CameraInfo error→production paste/Retry→distinct fresh
CameraX/ZXing QR→closed behavior. Original policy is restored and checked through
the actual native getter in finally. If this SDK only blacks frames rather than
emitting CameraState error, `unsupported-error-trigger` is blocked/nonzero—not
async-error proof or a fake CameraState fixture. Android14 command source:
[SensorPrivacyService shell](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-14.0.0_r1/services/core/java/com/android/server/sensorprivacy/SensorPrivacyService.java);
this is API-source evidence, not an installed-image byte-equality claim.

Production Retry now binds native error authority to its fresh owned binding:
the shared initial CLOSING/CLOSED snapshot is not a new-attempt failure until
that binding observes PENDING_OPEN/OPENING/OPEN or receives its own real frame.
Thereafter EVERY live error, including CLOSING/CLOSED, is delivered through the
existing route/binding disposal guards; provider/bind/analyzer exceptions still
fail immediately. The guard regression
`cameraRetryOwnsFreshErrorsAndDisposedBindingsCannotFailTheNextAttempt` exercises
the observed stale CLOSED/error5 → fresh OPENING sequence, all live error states,
fresh frame authority and queued binding/route disposal. These synthetic state
objects are guard regression inputs, not actual hardware/error acceptance proof;
the unchanged native policy-error/Retry/fresh-QR flow remains the end-to-end gate.

The sensor-policy help probe passes its already captured and independently
rechecked actual guest USER_ID: this SDK parses that argument before dispatching
even `help`. Read-only help inspection accepts only its known native exit255
or documented0, empty stderr, unchanged bounded capture and exact command lines
`help`, `enable USER_ID SENSOR`, `disable USER_ID SENSOR` before any policy effect.
That is not state-changing success: enable/disable and all other commands still
require normal successful exit, with actual native getter/state restoration proof.
There is no hardcoded user, failed-command retry, assumed API support or fallback.
SDK command failures retain only safe operation/exit/byte-count/bound context in
the existing selected-mode report; extra events and all raw output remain absent.
The closed existing ACK protocol now includes first-qr, neutral-after-back,
policy-enable, policy-restore, second-qr and policy-original. No host policy or
physical camera is changed. Actual scoped evidence and remaining original-host
prerequisites are recorded in the [status](STATUS.md); every replay still
requires its own matched current source and ownership proof.

Actual writable/default-admin/user-unit acceptance runs only in a separately
pre-provisioned disposable login with its own real unchanged runtime/user manager;
the recorded workstation production UID1000 is refused. No sudo/user creation/
global package or production installation is performed. In that account:

```sh
nix develop path:. -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- --setup --disposable-uid "$DISPOSABLE_UID" --goatr "$GOATR_BIN" --config "$GOATR_CONFIG" --workspace-root "$WORKSPACE_ROOT" --evidence-dir evidence/setup-service
```

This calls actual packaged setup in a private PTY with separate finite consents,
asserts absent/decline/cancel/identical canonical-ID preservation and unit/start
declines, then proves actual protected devices.list + unit PID/argv/cwd and real
default pair/hello/snapshot/devices/revoke. Config/unit/identity FD/bytes and process
epoch must survive rerun unchanged; no setup admin override or second database/
identity owner is invented. Stop targets only the still-proven owned unit;
config/keys/receipts/unit records remain, not a final product removal claim.
Before starting that installed/start-declined base unit, the disposable runner
adds an exclusively created unmanaged `goatr.service.d/override.conf` containing
real coreutils canary `ExecStart`, `ExecStartPre` and `ExecStartPost`. Actual setup
must refuse both before and after daemon-reload, with no canary/default identity/
daemon effect; only the exact test-owned override is then removed. Setup scans
actual manager `UnitPath` named/type-wide drop-ins, checks `DropInPaths`, exact
fragment/backing content and `NeedDaemonReload` initially, after reload, after
start consent and immediately before start. This is a runnable disposable-login
gate, not an exercised manager result on the workstation or sandbox.


Native inventory inspects exactly all ten intended native entries, rejects extras/
desktop natives/JAR and compressed APK libs, checks every ELF LOAD and actual
zipalign16-KiB offsets, independently retained nonnull shared four identities/
secure selections, complete generated API/context/native producer hashes, exact
six Maven copies and pinned NDK28.2 `llvm-strip --strip-unneeded` transform of
four raw FFI/JNA members, actual debug signer and packaged publisher/source/
libyuv/license terms. It never asserts raw producer==APK or ignores added JNI.
SDK tools and actual cached verified AARs are prerequisites; explicit
`--native-output`, `--jna-aar`, `--camera-aar`, `--graphics-aar` resolve ambiguity.
Static alignment does not prove native use or real16KiB runtime.


## Isolated smoke ownership

Each run records UUID S plus exclusive short random suffix s. Use a private root `<workspace_root>/.gs-<s>` for project/integration/test state, where `<workspace_root>` is the deployment's configured wrapper-accessible workspace directory (for example `~/Workspace`). Exclusively create same-UID 0700 `$XDG_RUNTIME_DIR/gs-<s>/` and use its `admin.sock`; owned Herdr sockets also live there (or in source-supported canonical `~/.config/herdr` placement). Launch `goatr serve --admin-socket "$XDG_RUNTIME_DIR/gs-<s>/admin.sock"` directly, not through setup/production service installation. Pass that identical `--admin-socket PATH` to **every** `goatr pair`, `goatr devices` and `goatr revoke DEVICE_ID` invocation, including fault/restart/cleanup steps; no omitted override or fallback may contact production. **Never replace XDG_RUNTIME_DIR**: preserve the deployment's actual user runtime directory (normally `/run/user/<uid>`) and verify wrapper access. Derive host/provider-visible integration pairs through config; use `c.sock` and short relay socket basenames, preflight all socket UTF-8 lengths≤107 before effects. Herdr name `goatr-smoke-S`, unit prefix `goatr-smoke-S-`, owner unit `goatr-smoke-S-codex.service`; production paths/units never reused.

Transport-only modes run real packaged companion and second real iroh client; locally pair, hello, host.snapshot and authenticated notifications.read/sync where an event exists, or self-unpair for an empty host. **pair→hello→host.snapshot→device.unpair is the minimum successful framed authorization/action exchange**, not a provider control claim. They do not require credentials or create agents. Provider modes separately create an owned named Herdr server/project, actual desktop OMP/Codex wrappers and tiny prompted turns. Inject an owned already-bound loopback relay port into OMP runtime and overlay before launch; production remains7466. Never use a live production app-server merely because one exists.

Before cleanup, persist exact created endpoint identities, units, Linux boot/PID-start/socket inodes, Herdr name/socket, admin socket/private-parent path and ownership, runtime/config/state paths and emulator ownership in the run manifest. Cleanup targets only those still-matching owned resources; never wildcard-kill or delete pre-existing state. Service failures leave safe diagnostic evidence, not unrecorded leaks. Missing credentials/signing/ADB/host broker/user-service/public relay/ntfy prerequisites are named blocked gates; do not copy credentials, weaken wrappers/TLS, or substitute mocks.

### Plugin registry, roots and unit isolation

`--herdr-plugin` owns one phase8 lifecycle gate within the existing smoke example. Before any registration/install, exclusively create a private run root and set scoped **absolute** `XDG_CONFIG_HOME`/`XDG_STATE_HOME` for its Herdr and Goatr processes. Record Herdr's actual release/debug app directory (`herdr`/`herdr-dev`), registry/config file identities, managed checkout generations, plugin config/state, Goatr config/state/install lock, final retained Nix output roots/store paths, activation pointers and exact source commits. The pinned `config::io`/plugin paths source supports these XDG roots; `HERDR_CONFIG_PATH` alone is not registry isolation. Assert actual resolved paths lie within owned roots, and no production registration/enabled state/default profile is read or changed. Keep the real XDG_RUNTIME_DIR unchanged and preflight wrapper-visible paths/Unix-socket lengths.

Full installer setup/service/pair tests require a **pre-provisioned disposable Linux login/test environment**, with Herdr/plugin/companion under the same test UID, its own real unchanged XDG_RUNTIME_DIR/user manager and available actual installed wrappers. Do not create users/install Nix/use sudo during smoke. The production-default Goatr admin socket must be absent/unowned by any pre-existing process in that isolated account; otherwise block before effects. This exercises the real setup-generated service and ordinary pane pairing without inventing a setup admin-socket override or contacting a deployment user's daemon. Existing direct-launch smoke still uses its owned `--admin-socket` on every admin invocation. Service units retain actual packaged names in this exclusive account: reserve/record exact unit names, actual user-manager lookup/link destination, unit path/content identity, `ExecStart`, effective private config/state and Linux boot/PID-start before testing. A client XDG override alone does not prove the user manager loads isolated units/environment. Pre-existing/declarative unit conflicts block; never adopt them or claim unit-prefix ownership by name alone.

Use a disposable local filesystem Nix store/environment for the actual GC-survival subtest, or record that subtest blocked; never run GC or mutate store settings on the workstation's production/shared store. Build roots remain outside managed plugin checkouts. Record the registered root's **absolute final link path**, its resolved output/closure and all active/previous/independent-unit references. Before cleanup record the exact registry entries, root links/pointers, unit/process/checkout leases and private config/state owned by the run. Stop/remove only still-matching owned serve/test processes/units and registrations; remove only their proven unused output links/private test data. Never manually delete store paths, edit profiles, wildcard-remove registries/units or kill Herdr/providers/TUIs merely to release roots. Fixture preservation of pre-existing external installs and independently alive services must be evidenced before the runner's separate owned-resource teardown.


## Required behavior checks

Each row below names a runnable acceptance owner from the command suite and observable evidence. The Rust integration-test paths are proposed owners, not assertions they exist or a mandate for speculative files; existing module-local tests may fulfill the same ownership if mapped explicitly. Preserve every named gate when arranging the implementation.

### Herdr plugin installation and lifecycle

Owner: phase8 packaging/integration, `companion/examples/smoke.rs --herdr-plugin`, with focused fixtures in the existing companion package for source/ownership/activation transitions. Manifest/source fixtures supplement, not replace, **real** pinned Herdr CLI/terminal, GitHub install, Nix build/GC, Rust setup/service/admin and framed authorization evidence. Record exact deployed Herdr/Nix versions/source provenance, commands/exit status, reviewed plugin/package commits/locks, private registry/root/unit identities and sanitized observations:

1. Parse `plugins/goatr/herdr-plugin.toml`, discover `goatr.install-setup` and `installer`; verify Linux/min0.9.3 acceptance and no build/startup/events/link-handler commands. Exercise an actual too-old binary and unsupported host when claiming those runtime gates; fixtures alone are labelled fixtures. Check deployed `plugin.list`, action/list/invoke, pane/open schema/argv/env against the immutable source baseline; a0.9.3 label alone is not a pass.
2. Real local `plugin link` and public `plugin install --ref <reviewed-full-commit> trevarj/goatr/plugins/goatr` register only in the isolated registry. Record requested/resolved commit and package provenance, offline registration, explicit remote source/command confirmation, local-link overwrite refusal and no companion side effect at install/enable/server startup. Noninteractive `--yes` is permitted only for the already-reviewed isolated source. Missing public release artifacts block that gate, not an invented SDK/API prerequisite.
3. Invoke the actual action through the intended running Herdr session. It uses injected `HERDR_BIN_PATH` and opens the declared interactive overlay; observe a real terminal accepting install/setup input, visible progress/results/errors and prior focus restored on dismissal. Action/log command launch is not install success. Cancel/decline/pane-close before consent has no package/unit/key/config/pairing effects; cancellation after effects reports its exact completed stage, terminates owned children, releases the installer lock and preserves the previous working state.
4. Exercise missing Nix, unsupported version/features/local-store permissions, unavailable Git/network/build inputs, unsafe roots and unit permissions. Fail accurately with no sudo/tool/profile/cache/trusted-key/provider/global-setting mutation. A compatible external/declarative installation remains usable without Nix; incompatible/manager-owned conflicts are diagnosed, never overwritten/adopted/shadowed/removed. Test actual build/setup failure as well as deterministic stage fault injection.
5. Fresh consented build records the exact final `--out-link`/output suffix/store artifact/root registration, invokes the same packaged absolute `goatr setup` and selects verified wrapper-accessible paths. Declining optional serve start leaves no running daemon/window and gives usable absolute serve→pair instructions. Separately consented owned user service reaches real local health; explicit pane pairing plus a real iroh client completes **pair→hello→host.snapshot→device.unpair** (minimum authorization/action exchange). No hidden phone installer/admin operation. Do not capture QR/token in evidence: inspect actual Herdr action/build/plugin logs and installer records for secret absence, without persisting the matching secret.
6. Rerun preserves host/project IDs, keys, paired state/config/receipts and identical owned units/artifacts; no duplicate daemon/admin socket or competing key store. Simultaneously invoke across panes/named servers: one owned install lock covers effects, the other reports busy, and a later invocation re-reads source/ownership rather than clobbers completed work. Dirty/local development builds are explicitly disclosed/isolated and cannot replace production release ownership.
7. Explicit pinned new-release plugin reinstall leaves old panes on old files and causes no companion change by itself. The requested companion upgrade builds/preflights a candidate while the previous working artifact/unit/config/state stays valid. Exercise failure/cancel/crash before/after root creation, setup commit, pointer/unit/record transitions and service health: previous working installation is preserved/restored and rerun reconciles actual ownership. State-format incompatibility blocks promotion, never silently migrates destructively. Old/stale installer cannot implicitly downgrade; equal-version/different-source ambiguity does not masquerade as an upgrade. Restart declined reports staged, not live.
8. In the disposable Nix store, ordinary GC retains active/previous/all independently in-use closures at their **unchanged registered absolute root locations**; a separate atomic activation pointer is not mistaken for a GC root. Plugin checkout/generation cleanup cannot invalidate those executables. Herdr/provider/owner/TUI/Codex-relay PID/start/unit/executable bindings survive serve/plugin exit/update/restart unchanged; exact absolute `ExecStart` and cwd/config must not reference the plugin checkout. No hidden lifetime propagation or implicit relay restart/pruning.
9. Disable/unlink/uninstall prevents future entrypoints according to pinned source, preserves existing pane leases and independent companion/service/data, and triggers no imaginary uninstall callback. Separately consented companion removal proves owned unit/path/content/store-root identities, stops/removes only owned serve resources, preserves keys/devices/config/receipts/records by default and reports independently in-use roots retained. External/declarative installs and desktop sessions remain untouched. Purge is another explicit local choice, never inferred from plugin uninstall.
10. Cleanup evidence enumerates exact registry entries/root locations/unit/process identities removed or intentionally retained, and demonstrates untouched production registry/default profile/socket/provider settings and preserved external fixture resources. Unavailable deployed-API/source match, reviewed public releases, isolated login/store/user-manager/wrapper permissions or real GC/service/network evidence are named **blocked**, never mocked/skipped into success.


### Protocol fixture parity

`companion/tests/protocol.rs`, Kotlin `ProtocolFixtureTest`, shared `protocol/fixtures/`:
- Every method/result/error/event/type union, malformed/unknown behavior, framed Unicode/partial reads, recursive duplicate keys (including allowed nested maps), surrogate/byte/member/depth bounds, UUID/Uint/timestamp and decimal-string rules. Include raw-token negative-zero/exponent/noninteger rejection and rejection of booleans as integers before conversion.
- Shared canonical bytes/SHA256 and fixed-key HMAC vectors; method/default/field-order/array-order changes; exact answer-choice membership; no raw numbers leaking from native MCP forms. Prove the independent Rust/Kotlin canonicalizer matches key/escape/integer/default semantics; exact normalized decimal comparison/native-token emission covers the full128-byte bound and preserves integer/string JSON-RPC IDs without floats.
- Auth/hello/receipt lookup/stale boot/target/dispatch check order; same-ID/same-hash lookup produces zero writes, changed payload conflict, old-boot status recovery, peer isolation and all timeout outcomes.
- Snapshot response precedes seq1; duplicate/gap/overflow resync, reconnect replacement, bounded history/content/page/inventory/form capacity and error mapping. No silent dropped request/history transition.
- `companion/tests/pairing.rs`/`PairingTest`: exact admin NDJSON/credentials/private socket bounds; identical `--admin-socket PATH` handling for serve/pair/devices/revoke, unchanged omitted-option production default, rejection of relative/oversized/symlink/wrong-owner/nonprivate paths and wrong peer UID on client/server, absent overridden serve without fallback or second identity owner; wrong/expired/reused token, response-loss→known hello, unknown hello rejection, attempt limits, duplicate connections, deviceName/token lengths, restart expiry, scan/paste malformed input. Assert **zero session reads/provider operations before authorization**.

### Revocation fence

`companion/tests/security.rs` plus `--example smoke -- --backend ...` fault points:
- Pause immediately before provider write, every creation stage and push attempt; revoke then release: no subsequent native effect/publish. Already-started bounded write may settle, desktop work remains alive; stuck boundary yields uncertain and cannot resume later.
- Admin completion waits for earlier write boundaries; self-unpair emits only its final response then closes; queued subscriptions/receipts/notifications inaccessible. Peer A cannot query B receipts/read-watermarks.
- Full host authority: two supported sessions inside/outside configured project list both appear and allow history/control/notifications for an authorized peer, neither for unknown peer; new creation outside configured projects rejected.

### Native libraries

`nix build`, `:iroh-jvm:test`, `NativeLoadTest` instrumentation, `companion/examples/check_native_libs.rs`:
- Android artifact manifest proves one FFI source/patch/lock across its native outputs/generated Kotlin, full Kotlin-facing API compiles, and Rust companion compiles against direct iroh. Compare exact selected core/base/relay1.3.0 version/source/checksum or immutable revision and intended secure Rustls selection across independently retained companion/FFI locks/manifests; full locks/features and host/Android ABI need not be identical. Both resolved graphs (including duplicates) exclude GHSA-7cq4-mhxw-xw78/GHSA-jx4g-cg2x-jc35 and Rustls meets >=0.23.45. Record exact dependency/features/TLS/system-SQLite provenance, license evidence and advisory database identity/date, not version-name inference. Any host FFI is a JVM-only test input, not a companion dependency.
- Inspect APK ABI/native inventory, JNA AAR exclusion of desktop natives/JAR, Rust and JNA ELF/APK16-KiB alignment. Load both libraries and Android JNI context on owned emulator.
- Document API/ABI/device page size. Static alignment is not proof of running on a16-KiB device; unavailable real16-KiB runtime stays explicitly unexercised. Record copied-source notices and reproducible build input/license checks.

### Direct and relay transport

`--example smoke -- --transport direct|relay`:
- Direct uses endpoints without relays and explicit local address hints, observes selected direct path. Relay uses public N0 plus core `clear_ip_transports` on both endpoints (direct Rust call; patched binding when Android is the peer), observes selected relay path and completes identical framed authorized exchange. The transport-only second client also uses real direct iroh, not a host FFI requirement.
- Source reference is core1.3.0 `endpoint_two_relay_only_no_ip`; no firewall changes/insecure TLS. Record actual selected-path events, not configured intent. Relay outage is blocked.
- Network callbacks/path watches/streams release without leaks; endpoint recreation retains identity; duplicate phone connection policy has one active subscription set.

### Provider source parity

`companion/tests/omp.rs` and `companion/tests/codex.rs`, shared sanitized pinned-source fixtures:
- OMP registry privacy/bounds/races/duplicate processes; link normalization (including nested fragments/loopback), AES envelopes, hello/welcome/final snapshot, persisted-versus-transient boundary, rich-file versus placeholder merge, optional title slot/partial tail/replacement/truncation, unknown record handling, select/editor/cancel and room-recreation dialog loss.
- OMP loopback relay wrong path/role/Host/Origin, nonupgrade, duplicate-host ownership, guest IDs/routing/room close, frame/queue overflow and configured test URL parity.
- Codex every finite method/decision/question/MCP field type/constraint/default/native answer map, integer/string native IDs, offered grants only, session/policy labels, secret exclusion, unsupported forms/userVerification. Command fixtures must include the pinned **experimental runtime** fields: present restricted/reordered `availableDecisions` with exact amendment payloads, rejection of any unoffered base/amendment answer, empty list with no invented choices, malformed/unknown decisions→desktop-only, and absent-list defaults for network context (first allow proposal only), additional permissions, and plain/execpolicy requests in exact source order. Exercise precedence when fields coexist; never substitute defaults for a present list. Preserve full `additionalPermissions` and scope or make the whole request desktop-only for unsupported/oversized details. Include malformed/oversized fields and browser-open-not-resolved.
- `companion/tests/codex_relay.rs`: atomic manifest validation, socket/path/privacy bounds, owner/PID reuse, ready without consuming TUI slot, initial timeout, connected/disconnected/terminal_closed, bounded pending correlations, successful unsubscribe, unsolicited thread/started ignored, companion-independent relay/owner lifetime. Exercise the same packaged `goatr codex-relay --binding PATH` binary in its own service, not a second helper executable or `serve` child.
- Stable generated schemas omit experimental command fields and cannot alone define these source-parity fixtures; use the pinned runtime params, producer, decision conversion and legacy-default implementation linked in [References](REFERENCES.md#critical-files--anchors). Source fixtures still do not prove stock behavior: live provider gate separately checks loaded-list pagination, resume notification ordering, replay/first-response, native history pagination and busy turn/start.

### Live session and creation

`companion/tests/config.rs`, `companion/tests/herdr.rs`, `--example smoke -- --backend omp --backend codex`:
- Config schema/defaults/unknown keys/path ownership, project canonicalization and wrapper access, managed/symlinked conflicts, repeat setup preserving identity, optional service decline→foreground serve; one unavailable backend preserves the other. Owned smoke records unchanged XDG_RUNTIME_DIR and the same owned `--admin-socket` argument on serve and every admin command, proves pair/list/revoke and companion restart target only that socket, and leaves production admin socket/state untouched. No remote/TOML admin-path configuration or setup override is required.
- Record actual Herdr shell resolution to the installed wrapper; no fictional executable launch arg or unverified env.PATH guarantee. Fail creation preflight if proof missing. Prove namespace path pairs reference the intended object and sockets satisfy byte bounds.
- For each provider record owner PID/start, native conversation, Herdr incarnation/terminal, final binding and desktop geometry/focus before/after. Desktop and mobile prompts appear in the same native session without another engine/TUI or focus theft.
- No-focus creation returns ready target only after exact desktop binding. Unknown stage outcomes expose resources; exercise crashes before/after each side effect/result persistence, including workspace/thread/relay/TUI. Never second thread/launch, silent rollback or cleanup of user's work.
- Existing ambiguous/uninstrumented panes produce non-mutable UnboundRow; duplicate native IDs across owners, server restart/PID reuse and routing movement are distinct cases.

The focused Herdr tests
`creation_admission_uses_current_same_source_root_and_rejects_changes` and
`creation_shell_admission_rejects_changed_original_epoch` cover creation
admission across an unrelated completed topology revision, rejection while dirty,
changed owned root/focus/agent/source lineage, and original shell epoch/namespace
loss. These component checks do not replace actual no-focus creation for either
stock provider or prove which guard rejected a historical generic `not_ready`.

### Provider dialogs and cancellation

`companion/tests/omp.rs`/`companion/tests/codex.rs`, `ProviderFormsTest` instrumentation, real-provider smoke:
- Native approval/questions resolved from each side, simultaneous/late duplicate response closes once; stale key cannot affect newer request. OMP sequential select/editor/cancel, current-work Stop and desktop-only custom dialogs.
- Codex stale-turn cancellation rejected, offered session/policy grants exact, permissions scope explicit, secret input never stored, MCP standard types and explicit URL flow. Real first-response outcome is honest dispatched/unknown, never falsely this-phone-won.

The ordinary `sleep 30` Stop scenario waits, within its existing bounded
scenario deadline, for one snapshot of the original Target showing both
`Working` and connectivity `Ready` (plus the observed native turn for Codex).
It then submits exactly one Stop envelope. A rejection retains only its finite
`WireError.code` and that already-observed connectivity/working state; it never
retries Stop, retargets, or changes native abort/Busy authority.
An OK response envelope is not a successful Stop receipt: OMP requires its
documented `dispatched` write outcome, while Codex requires `succeeded` from the
correlated `turn/interrupt` response. A `reserved` or `dispatching` initial
receipt is observed only through `ActionStatus` for that exact original Stop
action, within the same scenario deadline. Failed, uncertain, unexpected or
erroneous receipts stop with finite state/code evidence before pagination. After
an accepted receipt the helper must observe the same Target `Ready` and `Idle`
within the original scenario deadline; no additional Stop is sent and no OMP
acknowledgement is invented.

The periodic OMP registry observation keeps its existing1500-ms deadline. A
deadline does not replace the authenticated source: it enters same-source
observation-pending, keeps reader/history/dialog evidence alive and rejects
mutations until one complete fresh same-binding registry/owner/control/relay
observation succeeds. All other registry and native-reader failures remain
fail-closed. This behavior requires fresh foreground proof and does not replace
the final both-provider Android acceptance.

### Reconnect and generation replacement

`companion/tests/state.rs`, `SessionRecoveryTest`, provider/UI smoke:
- Join mid-turn/pending request; native reconnect replaces generation and final snapshot precedes queued events. Owner/thread/terminal replacement changes sessionId; room replacement/reconnect keeps proven sessionId but replaces generation; pane routing alone does neither.
- Companion boot invalidates all prior mutation contexts/handles. Old draft/request/content/cursor cannot retarget; file-history/newer-live precedence holds under racing pagination. Lost OMP room requests remain desktop-only, never replayed.

### Mutation receipts and companion restart

`companion/tests/receipts.rs`, `companion/tests/creation.rs`, provider crash-point runner:
- Fault every reserved→dispatching→native-write→receipt commit window, including pre-write crash. Drop reply and restart; action.status yields recorded outcome/uncertain, never automatic duplicate.
- Only definitive no-effect is failed; ambiguous write or unknown create ID uncertain; OMP no-ack remains dispatched. Same-ID lookup after stale boot is harmless, changed payload conflicts, unknown/expired receipts never authorize resend.
- Capacity/30-day retention, secret original/answer absent from SQLite/logs/drafts, keyed fingerprint fixtures, pending120-second create response and later action event/status. Revocation stops subsequent stages not already-started work. Companion restart leaves desktop owner/TUI/Herdr running.

### Content, cache and privacy

`companion/tests/content.rs`, `companion/tests/notifications.rs`, Kotlin `SessionStoreTest`/`RecoveryDraftTest`:
- Exercise **every bound/eviction row** in protocol with boundary and +1 values; multibyte byte offsets, TTL/LRU/generation expiry, source truncation, cursor invalidation, oversize form desktop-only, cache stale/partial markers.
- Offline required cached history, rotation and process-death recovery, local pre-send receipt does not erase draft, no retarget/automatic resend, protected-key loss fails closed. Secret answers/original pending JSON never enter persisted metadata or diagnostics.
- Offline Forget removes all host-local private/cache/push/dedup state without claiming server revoke; Android backup/transfer exclusions cover all databases/files/keys. Attention sequence survives restart/rotation, watermark per-session/per-peer, retention gap reset, live/push dedup and replay suppression.

### Compose behavior

Execute **`:app:connectedDebugAndroidTest`**, not merely compileAndroidTest. `NavigationTest`, `ProviderFormsTest`, `SessionRecoveryTest`, `PairingUiTest`, `NotificationEntryTest` use actual Compose runtime/semantics/testTagsAsResourceId:
- Narrow/landscape/IME/multiple forms/font scaling, session selection/create/partial states, host-wide authority disclosure, no-project/ambiguous/setup/offline states, native Markdown/code backing-text copy/tool expansion/content refresh. The dedicated TalkBack audit is deferred for the first APK under the [handoff exception](#first-testable-apk-ui-handoff); existing semantics remain.
- All finite forms, option validation/scope labels/secret lifecycle, reconnect disablement, scanner denied/no-camera/Back/paste races and explicit-link safety. No source-text assertions/mock echoes standing in for UI.

### Emulator and camera evidence

`--example smoke -- --ui --serial ...` owns resources, installs/launches real APK and sends/stops through visible UI against real companion/providers. Save fresh PNGs for pairing, all-branches OMP conversation/tool cards, Codex conversation/forms, partial/uncertain creation, cold/warm notification entry and offline cache. Correlate screenshot timestamp with commands and source/native identities.

`QrDecoderTest` covers generated QR→padded/rotated camera planes and frame-finally closure; scanner instrumentation exercises lifecycle/cancellation and posted-callback disposal. Report actual camera input separately: paste and decoder fixtures do not prove camera hardware input. Never use a physical phone without authorization.

### Background delivery

`companion/tests/push.rs`/`companion/tests/notifications.rs`, `PushStateTest`, explicit `--example smoke -- --background persistent|ntfy --serial ...`:
- Persist-before-register; probe before response; process death before/after persistence; duplicate probe/ACK/ACK-response loss; deadline restart; stale unregister/ACK/generation; endpoint rotation and old-key rejection; no handoff before confirmed ACK.
- Event/probe payload alternatives (probe has no session), HMAC/base64/identity/size validation, same-ID live/push/retry dedup, durable sequence/read reconciliation, source completion versus disconnect, peer scoping/retention gap.
- Approved-origin HTTPS validation, query preservation, redirect/DNS rebinding/private-target denial, TLS error/no insecure bypass, HTTP429/5xx/404/410/auth responses, retry horizon/queue bounds and revocation cancellation. Deterministic HTTP fixtures prove failure policy; real ntfy proves distributor delivery.
- Real persistent attention and ntfy probe→handoff→attention→click→current-state flow, endpoint recreation/network changes with retained identity, permission denial/force-stop/Doze/service restrictions and visible repair without silent mode changes. Record API level and externally induced limits; never claim these platform/network limits eliminated.

### Security review regression-first gate

These additions are **test-only and unexecuted at handoff**. They deliberately
assert the corrected contract against the current APIs before semantic fixes.
Package4/5/6 and real OMP/Codex/provider effects are not exercised. Both Android
provider variants below are **synthetic authoritative provider-data fixtures**,
not claims of stock-provider or foreground UI acceptance.

Rust module-local selectors (the private entered-ticket/limiter harness is
necessary; no production test hooks were added):

| Finding | Exact selectors under `pairing::tests::` | Original defect |
|---|---|---|
| Full SQLite queue loses sole ticket | `review_queue_full_retains_owned_unix_dispatch_ticket`; `review_queue_full_retains_creation_socket_ticket`; `review_queue_full_retains_synchronous_start_ticket_and_outcome` | `held.take()` preceded fallible `try_send`; a full queue dropped the closure/ticket and synchronous outcome, leaving `done=false`. |
| Creation no-effect timestamp split | `review_creation_validator_failure_keeps_stage_and_receipt_timestamp_consistent`; `review_creation_unauthorized_keeps_stage_and_receipt_timestamp_consistent`; `review_start_validator_failure_keeps_stage_and_receipt_timestamp_consistent` | Settlement committed a failed stage using a newer timestamp, then rejected the older captured receipt transition with `invalid_request`. |
| Pending handshake budget not reserved | `review_endpoint_budget_counts_eight_pending_admissions_after_four_failures`; `review_global_budget_counts_pending_admissions_and_survives_endpoint_churn`; `review_pending_rate_reservations_expire_without_unbounded_endpoint_memory`; `review_successful_native_hello_releases_its_failure_budget_reservation` | Overlapping checks saw the same failure count; unchecked churn also bypassed reservation of the global allowance. |

The queue fixtures use real private file-backed system SQLite, its actual64-job
owner queue, actual owned Unix socket bytes, and a synchronous create-new OS file
admission. A real owner closure is gated; exactly64 further jobs are enqueued
while it is blocked. Capacity is then released, and the original completed
outcome, shutdown/revoke settlement, retained resources and zero later bytes
are asserted. Only read-only owner barriers repeat; native effects never retry.
For no-effect, the SQLite job is held until **the next actual Unix millisecond
after settlement enqueue**, not an assumed scheduling sleep. Both failed stage
and failed receipt/error must commit, with `done=true` and zero side effects.
The limiter concurrency uses eight simultaneous actual unauthenticated permits
and a barrier before they release; expiry uses paused Tokio monotonic time.
Success release additionally traverses real native TLS/framing/hello.

Run the retained regressions after integration:

```sh
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --lib review_
```

Android owns `io.github.trevarj.goatr.GoatrConnectionTest` with these exact method
selectors (append `#METHOD` to the class for individual instrumentation):

- `review_reopen_preserves_queued_old_frames_and_unsubscribes_omp`
- `review_reopen_preserves_queued_old_frames_and_unsubscribes_codex`
- `review_more_than_32_automatic_resnapshots_stay_bounded_omp`
- `review_more_than_32_automatic_resnapshots_stay_bounded_codex`
- `review_generation_changed_reopens_authoritative_inventory_target_omp`
- `review_generation_changed_reopens_authoritative_inventory_target_codex`
- `review_source_install_not_ready_then_ready_recovers_omp`
- `review_source_install_not_ready_then_ready_recovers_codex`

They use actual `GoatrConnection`, generated native endpoint bindings,
AndroidKeyStore and Android SQLite. The non-installed existing smoke example
supplies real Rust `State`/`Security`/`transport`/native endpoint/private SQLite.
Its loopback-only ephemeral controller accepts only closed, bounded synthetic
fixture transitions. Pair tokens are never sent through arguments or evidence:
the controller locally authorizes the test endpoint, and the consumer performs
a real known-peer hello. The host neither echoes frames nor forwards Goatr
transport; adb reverse carries **fixture control only**.

The original Android source removed old local routes without server unsubscribe;
later old-SUB events disconnected the host and repeated opens exhausted the real32
subscription cap. Recovery reopened the old generation once or abandoned
`not_ready`; later `host.changed` refreshed inventory without reopening the
session. The tests assert queued old frames remain acceptable until replacement
acknowledgement,40 actual automatic resnapshots remain usable, current inventory
Target recovers after installation readiness, disabled mutations create no
local pre-send reservation, retained receipts reconcile before final write
enablement, and old action/draft targets never retarget.
Executed failing-before/passing-after evidence belongs in [`STATUS.md`](STATUS.md).
There is no deterministic independent hold point between the Android snapshot
callback and a subsequent receipt response in the current public API; the
native tests assert final authoritative receipt readiness and pre-snapshot
write disablement, not an invented observation of that internal micro-window.

Lead-only native fixture procedure, using the existing caller-owned schema1
emulator manifest (do not create/adopt another emulator):

```sh
nix develop path:.#android -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- \
  --review-fixture \
  --emulator-owner /home/trev/Workspace/.gs-AEeNlL/run.json \
  --evidence-dir /home/trev/Workspace/.gs-AEeNlL/review-evidence
```

The fixture verifies private canonical manifest/root, owner UID, Linux
boot/PID/start/executable and selected `emulator-5580` console AVD name before
effects. It refuses an existing mapping, installs one `adb -s emulator-5580
reverse --no-rebind tcp:PORT tcp:PORT` for its actual owned `127.0.0.1:0`
listener, and records exact root/port/mapping/source-lock provenance in
`review-owned-RUN.json` and `review-ready-RUN.json`. Wait for the structured
`status=ready` line, then set `PORT` to its `controlPort`; N0 discovery/relay
unavailability is a missing runtime prerequisite, not provider evidence.
Install the already-built debug/test APKs only on that owned serial, then:

```sh
nix develop path:.#android -c bash -c '
  methods=(review_stateflow_observes_snapshot_before_live_subscription_event review_actual_subscription_sources_cache_and_navigation_follow_automatic_reopen review_real_content_reads_source_safe_cache_and_sqlite_reopen review_transient_activity_never_rewrites_completed_sqlite_cache review_actual_conversation_keeps_new_pending_forms_and_composer_reachable_at_history_top)
  for base in review_canceled_native_reads_validate_late_replies_and_reuse_slots review_native_read_timeout_keeps_late_reply_correlated review_canceled_native_open_cleans_only_its_returned_subscription review_late_native_history_guard_rejects_without_transport_teardown review_canceled_late_native_history_keeps_source_recovery review_reopen_preserves_queued_old_frames_and_unsubscribes review_more_than_32_automatic_resnapshots_stay_bounded review_generation_changed_reopens_authoritative_inventory_target review_source_install_not_ready_then_ready_recovers; do
    methods+=("${base}_omp" "${base}_codex")
  done
  selectors=(); for method in "${methods[@]}"; do selectors+=("io.github.trevarj.goatr.GoatrConnectionTest#$method"); done
  IFS=,; adb -s emulator-5580 shell am instrument -w -r -e class "${selectors[*]}" -e reviewControlPort "$1" io.github.trevarj.goatr.debug.test/io.github.trevarj.goatr.OwnedInstrumentationRunner
' _ "$PORT"
```

Select these23 `review_` methods only; the whole class now also contains the two
real-stock/OS-death consumers requiring `stockControlPort`, never fixture fallback.
Both stock `@Test` entrypoints explicitly use `runBlocking<Unit>` and export JVM
void methods. JUnit validates the entire class even when filtering to review
selectors, so a non-void stock method would block all filtered instrumentation
before any assertion runs. Rebuilt direct23-selector execution is the regression
proof; no wiring/not-throw fixture substitutes for it.

Tests refuse existing installation identity material, use exclusive test
database names, and clean only their recorded endpoint/host aliases and
fixture allowlist record, even on behavioral failure. Stop the fixture with
Ctrl-C (or its exact control `{"method":"stop"}`); its900-second lifetime also
bounds abandoned runs. It releases paused SQLite, closes native tasks/owner,
removes **only the still-matching reverse mapping on the still-matching
emulator lease**, checks its root inode before cleanup, and writes
`review-cleanup-RUN.json`. It never stops the lead-owned emulator or touches
production sockets, units, identities, providers or global settings.

**Hints publication before-proof limit:** the original `companion/src/main.rs`
made the final `create_new` hints path visible before serialization/write/fsync,
while the smoke consumer treated `hints.exists() && socket.exists()` as readiness
and immediately parsed it. The administration socket was bound earlier.
No public API deterministically pauses between create and write; therefore no
source-text assertion, replacement publisher, probabilistic race test or new
injection hook is presented as behavioral proof.
Exercise the corrected **actual packaged** producer and public smoke consumer
repeatedly:

Export `GOATR_BIN` as the absolute `bin/goatr` from the actual selected Nix
output. Compile the non-installed runner once, then reuse it for the batch:

```sh
nix develop path:. -c cargo build --locked --manifest-path companion/Cargo.toml --example smoke
nix develop path:. -c bash -c '
  for attempt in {1..50}; do
    companion/target/debug/examples/smoke --transport direct --goatr "$GOATR_BIN" \
      --evidence-dir /home/trev/Workspace/.gs-AEeNlL/hints-review || exit 1
  done
'
```

Every run must reach complete private hints/provenance parsing and the existing
real pair→hello→host.snapshot→device.unpair exchange without partial-JSON
readiness failure. Repetition is post-fix packaged smoke evidence, not a
deterministic failing-before proof of the create/write interval. Existing
unsafe-entry/no-overwrite requirements remain unchanged.
