# Current status

The approved [mobile MVP plan](UI_POLISH_PLAN.md) is active. Complete the full
feature matrix using one Sol 6.1 medium writer and independent Sol 6.1 high
review before signed commits. First integrate the functional component split
and create the authorized bulk source baseline; then commit between features.

Use incremental changed-behavior checks and the paired Mi 9T on Wi-Fi and in
background. Targeted regressions and device workflows are required for MVP;
exhaustive clean builds/release qualification remain later. The installed APK
below remains a development artifact, not a full pass. Host creation is blocked
by unproved coarse-shell evidence before Android; no new Create was submitted.
Preserve pairing, native identity, permissions, receipts and no-replay fences.

## Latest source changes

- Conversation **New session** reuses the configured-project form and receipts,
  pins the original desktop/provider and requires an explicit project/name.
  It creates a separate workspace; current-directory/same-workspace placement
  remains unsupported. Original draft, foreground/source gates and no replay
  are preserved. A reversed subscription lookup found in review was corrected.
- Main Sessions now has provider/status filters applied equally to live and
  retained summaries; cached rows stay visibly stale and retain exact Targets.
- Direct **Unread activity** from main-console host rows now reuses the secure
  host inbox, including offline hosts. Its synchronous handoff checks the exact
  paired host and foreground; listing does not mark entries read or sync.
- All three slices passed focused source review and are integrated in four Android
  files. Incremental compilation and startup passed in APK494f2465 below; individual
  feature workflows remain pending. Tests/lint/format were not run. Integration record:
  .goatr-creation-filters-integration-FRf5Cn; creation freeze-v2 manifest
  ee80673642e6178565905279e835a6ecc1dc42ae5c8b08279c012a1a497ea05c;
  filters manifest52da1639c1606bcb796778a6b4433076021105bb0c515491b0f7077cb6efef70.
- Console inbox integration record: .goatr-console-inbox-integration-zBTYhn;
  manifestd9ba6305cea2f164002de553f46349ffc189272692846ba52bf97be532c2626b.
  All coding/review agents returned before the incremental build. Next: changed
  feature checks, then remaining real workflows. Density refinement, tests and
  diagnostic-observer repair remain deferred.

## Installed development APK

[Current APK](../build/phone-features-494f2465/goatr-development.apk)

- SHA256494f24659a3b0382f9f8c66e01d62521afe2dd8bc92c7391f3c1ee36cec5d8a2;
  53,151,233 bytes; version0.1.0/code1; same debug package/signer below.
- Incremental Android build apk-console-features-468a5db7 passed16m8s:
  six tasks executed,53 up-to-date. Owner/runner and275 before+after source
  inputs passed; source4e1b9dc300a56ccba24513e81fb8b443bd0d1864d3c4a5b0e1d2ac9fb0c4e9bc.
- Install-r and normal registered launch passed, UID10083/data66320:1061713:10083
  preserved. PID15702/start23088784 resumed/caps0. Independent read-only
  observation69b919ef passed Sessions/Direct/notOffline with unchanged process.
  Records .goatr-console-feature-build-xdPhkn, .goatr-console-feature-delivery-oa3q9zzy,
  phone console-apk-observe-installed-controller-bYPziJ. Pairing works.
- Includes all three latest source changes. Individual workflow checks pending;
  original native/pairing guards passed around update/startup, no provider input
  or new creation submitted. Full acceptance remains incomplete.
- Private controller build8d0bf437 passed after pinned Java repair. Initial
  controller-only update lacked test-package -t; acknowledgement failed and an
  independent check showed old controller76206a5c still installed, app unchanged.
  A fresh corrected-r-t update passed; controller60294b58 installed with app
  PID/start, app/controller UID/data and original native/pairing preserved.
  Record phone controller-inbox-install-testflag-h6y3jm3k.

- Actual non-sensitive console screenshot shows provider/status chips, direct
  unread action and the original OMP row. Record console-feature-ui-7nmHEM.
  One raw ADB Codex-filter tap attempt returned device_read; independent
  screenshot still showed All providers. Effect unproved, no repeat. Record
  provider-filter-check-elqKy1. This is not a proved product-filter failure.
- Supported UI controller NavHosts76b7f1af and ViewSessionsb310bff3 passed Direct
  with original product preserved. Phone is on host-level Sessions. Fresh Codex
  selection is server14c91711/project2cdc437b/providerCodex/owner42ec31ec;
  receipt reconciliation/separate confirmation must precede any new creation.
  No new creation submitted. Older4d2 host receipt is Failed/not_ready; the
  actual displayed current receipt and fresh workflow are recorded below.

- Actual host-level receipt is f3c4b4e8-99cd-4a08-ab17-dc166af18d39,
  Uncertain/not_ready, distinct from older4d2. Read-only visible-tag discovery
  identified it after4d2 inspection refused before click. New independent
  workflow current-receipt-codex-flow-MnsNJF inspected f3c4, displayed details,
  prepared and confirmed separate creation, then selected server14c91711,
  Codex, project2cdc437b and name Goatr phone Codex6e937aa5. All actual UI
  controls passed; old resources/receipts untouched. No Create submitted.
- Fresh sheet is blocked because server canCreate.codex=false. Actual setup
  disclosure yielded coarse_shell_unproved (current shell executable/version,
  reporting namespace and integration views unproved), plus generic shared mode
  message. Record creation-reason-observation-AJ247n. Diagnose the existing
  coarse-shell proof failure before any creation; do not submit to probe.
- Controller-only a2e789a4 built/installed with original app/process/data/native
  preserved, adding OpenCreationDetails. Its one click acknowledged and actual
  disclosure opened; the helper's postcondition timed out because it checks
  text on the tagged button, while Material IconLabel places it on a child.
  No retap. Record open-creation-details-az6T4X. Current test controller is a2,
  not602; preserve current creation prerequisites from602 and explicitly restore
  the proven602 controller before using the unchanged Submit wrapper.

### Previous APK and retained evidence

[APK](../build/phone-features-1b0540f4/goatr-development.apk)

- SHA256 1b0540f4fbe25376971b5c20f44ad9668478132a815c6fcd5fec5a88d3b2551d;
  53,121,581 bytes; io.github.trevarj.goatr.debug, version0.1.0/code1.
  Signer9862aee51f34198b4b956e510856ad7ed3efe125b6e2522518cb945227af0300.
- Includes the previously compiled feature batch plus OMP inline Skills and
  host-scoped unread activity. Incremental build758125bc passed12m55s; actual
  owner/runner/before+after source proofs passed for274 inputs, source manifest
  SHAcc99953b0b1255e41af93650e0ca9d06e58176d24f975af5f574d77ae5e7e0b6.
  Matching native/host binaries were reused. Tests/lint/format, clean builds
  and exhaustive acceptance remain deferred.
- Installed on Mi9T74110723 with UID10083 and app-data directory
  66320:1061713:10083 preserved. Normal registered launch passed stable
  PID10055/start22755611, resumed/caps0. Independent startup observation460de525
  showed Sessions/Direct, not Offline. Existing pairing works after update.
- Records: .goatr-inbox-apk-delivery-yy_w68fp; sealed artifact record
  phone-feature-delivery-SP4zoS under the build evidence root; phone startup
  inbox-apk-startup-observe-MuE74f. Previous working APK50d48240 is retained;
  its fresh original OMP prompt/reply proof is recorded below.
- Matching host b56421a9/helper e814c700 remain activated. Reference
  paired-combined-upgrade-jt1qaeyi/new-original.json SHA
  27507bc3fa7650db3b02976f83818d9814f62b49261d847f353ca60293e347b7.
  Original OMP native mapping/generationd1266201 and pairing/keys were preserved
  across install/start. No stale request was retargeted.
- Unread-activity entry is visible on the current host Sessions screen. One
  validated local open tap was attempted; follow-up observation returned
  device_read. Product PID is unchanged and crash buffer empty; actual inbox
  state awaits a non-idle observer. Do not repeat the uncertain tap. Evidence
  inbox-ui-capture-K5apw6; the private two-action observer build exited1 before any installation.
  Observer repair is deferred while feature implementation proceeds.
- Current host storage has only the original OMP agent. Original Codex owner
  unit4cb885dc is absent; native root2563898/start257546436 and shell2563990
  remain. This is a native owner/binding prerequisite, not an Android failure.
  No historical cause/date or recovered original Codex continuity is claimed.
- Other feature workflows, original desktop continuation and both-provider
  continuity remain unproved. ntfy is absent; push delivery untested. General
  terminal Close, OMP Skill invocation and Codex document authority stay open.

Controller privileged exe/namespace/fd objects remain inaccessible; startup
namespace attestation plus fresh kernel/unit/argv/cgroup joins are explicitly
labelled process-only proof. Zero-cap native objects are independently checked.
No full acceptance claim.

## Original-provider evidence and known failures

- Previous APK50d48240 and matching host passed fresh original OMP
  mobile use: receipt5ca6df72-a1cd-4fc0-9116-8bb443588636 Dispatched/no error
  on generationd1266201; exactly one native prompt4965a455 and exact reply
  c71b3837 in the same original file/process/conversation. Record
  combined-mobile-omp-eeGWS8/mobile-effect-v3.outcome.json; current guards
  bracketed input and independent receipt/native verification. This proves
  mobile use, not desktop continuation, Codex use or a full acceptance pass.

- Previous APK0faa80fd sent one fresh mobile OMP prompt: receipt56f47cc5 was
  Dispatched/no error on original Target3120038e; native prompt18b97b09 and exact
  reply653dc6da occur once in the unchanged native history/file/process scope.
  Records shell-consumer-mobile-omp-9rAlBY/mobile-effect-v3.outcome.json.
  Earlier read-only verifier mistakes were corrected without repeating input.
- Desktop agent.prompt5eb46254 acknowledged on the original pane/socket, but
  independent native history and one bounded visible read found no marker/reply.
  This remains Uncertain: no resend or extra Enter. Source review distinguishes
  PTY-write acknowledgement from native consumption; the cause is unproved.
- APK0faa80fd foreground return showed Offline/NotReady. Retained expanded host
  diagnostics identified Timeout during Hello confirmation; failed connect was
  also masked by unconditional selection.open. The reviewed fix is in the APK
  above; the newer return succeeded. This does not prove every timeout resolved.
- Fresh current-APK Codex creation action4d2cac0f-7583-4ac7-bb7d-acdc3245c4f6
  was submitted once through the real phone flow (attachment-host-codex-create-bOOtSk).
  Workspace/Create confirmed, then ShellProbe failed/not_ready: a matched original
  shell query rejected resolution before provider launch. Exact kind/path/unsupported
  category was not captured in that phone run. A new isolated owned diagnostic
  4de102d8 captured the first guard shell_probe/result_rejected_path; capture drain
  completed with two closed lines and EOF. The guard proves resolved shell Codex is not the configured approved wrapper.
  A new fresh owned run codex-wrapper-path-QpAN1w is exercising a process-local
  PATH prefix to the unchanged approved wrapper. It failed at the same first guard;
  capture drain completed and the original helper exited. No provider launch, phone
  change or replay. NixOS /etc/zshenv was proved to reset PATH. A third fresh run729858bc
  used a private0600 .zshenv after that reset and succeeded: actual creation receipt
  0cd848b9-eca9-484f-ac74-d18930cc9be8 is succeeded/no error. It then failed
  after creation at the independent paging witness one-shot presence check,
  which conflates Past/Unavailable; a bounded same-binding freshness wait remains
  helper work, without relaxing identity or adding native probes. No phone
  creation/continuity or history-command pass is claimed. Failed-run resources
  remain retired; no adoption or replay.
  Retained w2/w2:p1/term_65e4e4bd803fa2 are failed-run
  resources, never adopted or replayed. This is a native blocker, not Android timeout.
- Original Codex mobile/desktop use, native creation blocker and full acceptance
  remain incomplete. Android foreground evidence does not resolve native failures.

An earlier APK51 screenshot exposed the attachment label wrapping one character
per line. Its narrow natural-width fix is installed in APK50d48240; current
visual verification is pending.

## Earlier feedback and unexercised features

APKbb3bda55 Direct/original OMP open and receipt-recovery gate observations are
historical; retained records are feature-batch-* under the phone evidence root.
APKf6496e20 draft2ab6 was unsent; SubmitPrompt5653d17e stopped before click on
inventory freshness. Its retired action is never replayed.
Saved routes, real notifications/entry, Changes insertion, bound checkout and
Skills native use remain unexercised. Native/Android builds do not prove them.

## Implementation and build queue

- Android-only incremental build apk-feature-retry-758125bc passed and is installed for
  integrated inline Skills/inbox. Executed node on
  .goatr-inbox-apk-build-corrected-gos4p8ot/launch-once.cjs; owner unit
  goatr-build-owner-758125bc-d448-4460-a7ab-83af2e50f263.service,
  invocation52b54c24a4e449c9a68187db46347a75, PID49326/start263168830
  revalidated in proc. Spec SHAcd6bad16ab5540187145c1713a507a01c938cf6d755e254e6ea3ea3ad8728ecf.
  Pinned Android drv, offline incremental assembleDebug, strict dependencies,
  Nix cores2/max-jobs1, Gradle max-workers2/no-parallel and process-local
  ActiveProcessorCount2; matching native/host reused after current-byte proofs.
  No clean/tests/lint/format. Actual owner/runner/source proof, signature, sealing,
  data-preserving install and normal startup passed; individual workflows remain open.

- OMP inline `/skill:` suggestions are source-reviewed and integrated in two
  Kotlin paths. They reuse the explicitly loaded original catalogue, offer at
  most six results and append command text without invocation or sending.
  Codex exact mentions are preserved. Handoff .goatr-omp-inline-skills-8ftd1nm7;
  integration .goatr-inline-skills-integration-IIzpQ4. Not in installed APK50;
  now compiled/installed in APK1b0540f4; catalogue workflow checks pending.
- Host-scoped unread activity is source-reviewed and integrated in the ViewModel
  and Screens. The secure inbox reads up to100 retained unread events, including
  offline/empty-session hosts; local refresh and exact-session routing reuse
  existing state without marking items read from the list or tap. Lifecycle and
  host changes clear the list. Handoff .goatr-attention-inbox-1qfd3nce; integration
  .goatr-attention-inbox-integration-lzsOlW. Compiled/installed in APK1b0540f4;
  entry visible, post-tap outcome pending without replay.

- Artifact browsing/export and agent-directory terminals compiled in native
  herdr-artifacts-1847d78e (7m55s), companion/helper
  goatr-artifact-compile-a8da4d29 (8m39s) and Android
  phone-artifact-syntax-30dc7eb5 (29m43s). Retained APKbd8a6062
  SHA256 bd8a6062bedf1afc2df90226ec52fbd22357457654a8e4c3ba0f9da20519d891
  crashed during normal startup: GoatrViewModel's immediate StateFlow collector
  accessed agentTerminalCreation before its delegated property initialized.
  The exact declaration was moved before init, source-reviewed and integrated;
  APK50d48240 now passed normal startup and Sessions/Direct observation. The
  failed artifact is not installed.
- UnifiedPush is integrated in the main tree:36 paths, including actual produced
  Android/Rust locks. Astra and mechanical source reviews passed the lifecycle,
  permission and durable attention fixes. Registration, verified push hints,
  native attention and notification entry compiled on the host and in installed
  APK50d48240; delivery remains unexercised. Read-only package lookup confirmed
  ntfy absent on Mi9T74110723; no other app was installed or changed. Existing
  persistent-service fallback remains available, with runtime checks pending.
- OMP Skills catalogue, draft insertion and complete secure document preview
  are reviewed and integrated:30 Goatr/11 native paths, exact baselines retained
  in .goatr-skills-integration-qty3JP and .goatr-skills-native-integration-3utAkF.
  OMP paths stay extension-local, actual native scope and original connection
  are checked at queued admission and after replies, canceled requests drain
  matching replies, and cleanup never holds the peer guard across await.
  Native incremental herdr-skills-821ab67a passed5m12s; sealed binary
  SHA2567560454f36b57c74b5559287cebe686b179184bc97bb5ba9f028c0d1bdb5d52a
  at sealed-native-skills-2N6pM6,2798 exact source inputs. Activated only on
  the fresh private Skills source recorded below; the matching paired controller
  is now activated.
  Serial pipeline feature-batch-248f5ebf stopped at four host compiler errors.
  Minimal three-path fixes passed source review and are integrated in
  .goatr-feature-compile-integration-v5smhQ. Fresh serial retry
  feature-retry-afd69cd5 host/helper passed9m53s. Its APK failed13m56s
  at one invalid PushCoordinator when-branch comma. The exact one-character
  correction and queued provider-neutral Skills copy passed review/integrated
  in .goatr-android-compile-integration-7HUXDx. Android-only retry
  apk-feature-retry-f4895fb5 passed17m28s and is installed. Rust/wire inputs
  were unchanged, so matching native/host binaries were reused.
  All274 product inputs matched sealing (only STATUS/UI docs changed).
  Actual sealed host b56421a99c33c1625589b31edad8446765c9f8b0f879142c0234f984ad3d12a6
  and helper e814c700a46307398c87141779926a48e803dfb63343b8edcab9311bed965eb9
  are at sealed-goatr-features-qsktr4 and activated in the new controller. Guarded phone delivery and producer alignment passed
  source review in feature-batch-phone-delivery-corrected-plyeryuy; device
  install and normal launch succeeded. First controller preparation21i26nvd
  refused before effects because expected argv omitted the legitimate relay FD.
  Fresh corrected paired-combined-upgrade-jt1qaeyi passed review/preparation and
  original-helper pidfd retirement/replacement proof. No live Skills proof. Existing OMP sessions receive no
  injected extension.
  Native invocation and Codex full-document authority remain incomplete.
- The observed Codex witness one-shot freshness failure has a reviewed,
  integrated helper fix, .goatr-witness-integration-ehJN5H. Only matching typed
  Past/Unavailable observations wait within original deadlines; all identity
  failures still reject. It compiled in this host/helper batch; its native path is unexercised.
- Next batch combines corrected Skills with integrated UnifiedPush, artifacts,
  agent-directory terminals and the startup fix. Use incremental two-core
  compilation only, one heavy build at a time; no tests/lint/clean build now.
  Keep working APK51f08d05 available until the new APK starts successfully.
- Astra confirmed remaining Skills prerequisites: OMP public expansion reads an
  unbounded pathname and public submission returns void, so neither proves a
  bounded correlated native invocation. Codex artifact OS-cwd scope does not
  establish accepted-thread read authority; fs/readFile is unbounded/sandboxNone.
  Keep invocation and Codex content unavailable until genuine native seams exist.
- General terminal Close requires transparent lifetime provenance and a
  protected atomic process-subtree broker absent here. No new OS privilege,
  numeric-PID scan or generic pane-close fallback is authorized.
- Paging beyond1000 is integrated; strict retention-crossing proof stays
  [deferred](PAGING_PROOF.md). The owned Codex creation path now succeeded in a
  diagnostic after a private .zshenv restored the approved wrapper after the
  NixOS global PATH reset. Its later one-shot witness freshness check failed;
  original Codex continuity and the historical invalidation cause remain open.
- Saved routes, real notification delivery, Changes insertion, worktree actions,
  attachments, terminal appearance and original Codex need functional exercise.
  Builds and source reviews do not establish those results.

Immutable earlier detail is retained at /home/trev/Workspace/.goatr-matrix-current-pyH20C/STATUS.md.
Main HEAD a55cd979cab6a0a47d4c757ac97b97d8535ee9eb. Current source companion lock
SHA256 564e08eb50f4ed88df20d6249e408b63e57a02aa8db0516aa7ff276f3a98fb00;
dependency provenance4fbfa3664e85614d7bfd9ace244b48cf90afaa404e05471b5ca3db388593575a.
Previously exercised host/APKs used locka638a212; no new batch build pass claimed.

## Original resources to preserve

Reprove current original identities before effects. Unit active alone is not
proof that its helper is alive. Never adopt retired failed-run resources.

- Current temporary controller goatr-phone-combined-a9a7a9b2-f15e-43f7-949d-4a4f7b96a788.service;
  invocation3c2db5a3e916465bb504a725e9b340c6; helper4169741/start262908291,
  Goatr4169986/start262909282. Config/proof in paired-combined-upgrade-jt1qaeyi.
  Hostb56421a9/helpere814c700; private runtime r-com-a9a7a9b2 and evidence
  e-com-a9a7a9b2 under the retained phone state root. Old32f78706 controller is
  retired, not current. Never execute its old controller-check module again.
  Paired SQLite dev57/ino34593482/invariant
  bd5b4a60ed4bca5cfb32da28da3628f46d305c9d1ec2bb67c073014e32352830 retained.
- Original OMP sessione6599041-a7c3-4213-82d0-162b77e3c04e,
  historical opened generation3120038e-f6c4-4448-ad91-21bf3262463c;
  prior-upgrade generationf4c5c9b2-dfbe-46b3-bf90-a4c3ed7f22eb explicitly opened;
  previous generation37b1bdc9-1dca-499f-87ca-c4fb4ba8e926 explicitly opened;
  current generationd1266201-03b6-461f-96bc-f46cd410c932 was explicitly opened
  and passed mobile prompt/reply on previous APK50d48240;
  native01a151d8-7275-772c-8d9a-54e311035b4e; owner3324493/start245208324;
  pane w2S:p1, terminalterm_65e27b322cfba3d. Default Herdr2975300/start57078518;
  socket /home/trev/.config/herdr/herdr.sock. Current guard combined-live-guard-co2mxp7m passed before/after the APK
  update and mobile workflow. Old attachment check module uses retired controller
  identities and must not run.
- Controlled native goatr-cdxv2-native.service, invocation850204cf82a349478f0edce62b4da555,
  PID2563898/start257546436; root /home/trev/Workspace/.gr-cdxv2. It is not
  upgraded to the new shell foundation. Current selected Codex owner has no
  proved new creation/use; earlier Launch/not_ready and history failures remain
  historical, not Android failures or a diagnosed original invalidation cause.
- Fresh private feature native server is live: native-features-pw1Jgx, unit
  goatr-features-native-d41fd47a-7e8b-4e53-9aa4-6c4318b6cee7.service,
  PID3614720/start260957776. Executable09be358a and owned0600 socket were
  independently verified; native parent/direct shell child have zero capabilities.
  Its local read_create_input grant applies only to this fresh isolated server.
  Original desktop servers remain untouched; feature behavior is not exercised.
  Matching controller is now independently proved running in
  paired-attachment-upgrade-9s7gJI; original owner records and paired state retained.
- Fresh artifact native server goatr-artifacts-native-a581362a-0bd5-46b0-8ec8-0152be2acfbe.service
  is live: PID3982186/start262235400 and direct shell3982254/start262235622,
  both caps0. Private config/socket and actual HERDR_CONFIG_PATH were verified;
  native-artifacts-prepared-77BhO5/live-proof.json. Independent artifact read and
  shell read_create_input grants apply only there. No feature methods exercised.
  Controller replacement is prepared in paired-artifact-upgrade-sNyjBV, not
  executed; current paired controller, all existing sources and phone are unchanged.
- Fresh private Skills native source goatr-skills-native-eee07006-d610-43d8-943d-7a2ce469a378.service
  is live: native4084227/start262603127, direct shell4084311/start262603501,
  both caps0. Invocation92fccc795e9b492e9e65ae018baa0d59; root.gr-gseee07006d6.
  Exact private config/zshenv/socket/sealed executable proof is in
  native-skills-prepared-T0N5mY/live-proof.json. Independent Skills preview,
  artifact read and shell read_create_input grants apply only there. No feature
  methods exercised; no compatible paired-controller transition yet.
- Existing user Codex owner793dd4d0 and other native work remain untouched.
  New desktop/native children must stay capability-free; only the already
  authorized temporary controller may hold CAP_WAKE_ALARM.

## Next actions and evidence

1. Implement the next feature-matrix slices in an isolated worktree, with one
   writer and focused independent review. Conversation-accessible creation
   provider/status filters and console unread access are integrated.
   Density refinement and observer repair wait.
2. After reviewed code returns, use incremental two-core builds and checks of
   changed behavior. Current APK1b0540f4 already starts Direct with pairing/data
   preserved. Tests, lint, formatting and exhaustive acceptance remain deferred.
3. Resume both-provider mobile/desktop/foreground checks without replaying
   uncertain input. Original Codex owner4cb885dc is absent; do not call a new
   session recovery of that original.

Build/native evidence: /home/trev/Workspace/.gs-herdr-report-Y0jW2m.
Phone evidence: /home/trev/Workspace/.gs-phone-mi9t-uXGRqx.
Native source: .herdr-mobile-shell-foundation-AIWuOl and .herdr-mobile-shell-fix-uJD82c.
Consumer source: .goatr-mobile-shell-consumer-8mogyj.
Prior detailed status is retained at /home/trev/Workspace/.goatr-status-current-mF2O3e/STATUS.previous.md.
A writer briefly changed nine main Rust files instead of its worktree; exact
restoration to the APK-start hashes was independently verified. APK inputs were
untouched (Gradle references only unchanged companion lock/dependency metadata).
Record: .goatr-input-directory-correction-PZ2pXL/root-verification.json. Future
mutations require explicit workdir and executable cwd guards. No native effect.
No commits, pushes, resets, global settings or receipt rewrites performed.
