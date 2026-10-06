# Testing and acceptance

Scope: the proposed implementation command suite, owned smoke resources and all required behavior/evidence gates. Each gate retains the original required check.

[Main plan](GOATR_PLAN.md) · [Main phase gates](GOATR_PLAN.md#implementation-phases-and-dependencies) · [Tech stack](TECH_STACK.md) · [Source anchors](REFERENCES.md#critical-files--anchors)

**Current status: planning only (2026-10-06).** No Goatr builds, runtime checks, linters, tests or formatters were run in this documentation revision. All commands, scripts, test classes and evidence outputs below are implementation obligations, not existing tools claimed to pass. Source inspection establishes only the facts labelled in [References](REFERENCES.md).

## Proposed command suite

Implementation must supply the named runners/tests using stdlib unittest, existing Kotlin/JUnit/Compose tools and the pinned Nix environment; no new orchestration framework. Run unit/native gates once after integration, then owned runtime gates:

```sh
nix build .#goatr .#iroh-android
nix develop -c python3 -m unittest discover -s companion/tests
nix develop -c bash ./gradlew :iroh-jvm:test :app:testDebugUnitTest :app:assembleDebug :app:assembleDebugAndroidTest --stacktrace
nix develop -c python3 tools/check-native-libs.py app/build/outputs/apk/debug/app-debug.apk
nix develop -c python3 tools/smoke.py --transport direct --evidence-dir evidence/direct
nix develop -c python3 tools/smoke.py --transport relay --evidence-dir evidence/relay
nix develop -c python3 tools/smoke.py --backend omp --backend codex --evidence-dir evidence/providers
```

Owned emulator/UI execution (runner refuses an existing/unowned device at the selected serial; it records the created AVD/process identity):

```sh
nix develop .#emulator -c python3 tools/smoke.py --prepare-emulator --api 34 --serial emulator-5580 --evidence-dir evidence/android34
nix develop -c env ANDROID_SERIAL=emulator-5580 bash ./gradlew :app:connectedDebugAndroidTest --stacktrace
nix develop -c adb -s emulator-5580 install -r app/build/outputs/apk/debug/app-debug.apk
nix develop -c adb -s emulator-5580 shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p org.example.goatr.debug
nix develop -c python3 tools/smoke.py --ui --serial emulator-5580 --backend omp --backend codex --evidence-dir evidence/ui
nix develop -c python3 tools/smoke.py --background persistent --serial emulator-5580 --evidence-dir evidence/persistent
nix develop -c python3 tools/smoke.py --background ntfy --serial emulator-5580 --evidence-dir evidence/ntfy
```

`--ui` uses real installed APK, Compose semantics/UIAutomator and real owned companion/providers, saves fresh `adb exec-out screencap -p` PNG bytes plus UI hierarchy/logcat filtered of secrets. `--background` drives visible mode selection, actual registered distributor/foreground service, generates real provider attention and exercises click/resnapshot; it cannot replace network delivery with a mock and call the gate passed. API34 default smoke is baseline, not proof of target36/newer restrictions. Repeat owned instrumentation/UI/background on API36 (or newer platform whose behavior is claimed), with `--prepare-emulator --api 36 --serial emulator-5582` and matching serial arguments; image/distributor unavailability is blocked evidence, not a pass.

Runner output is machine-readable gate name/pass/fail/blocked, actual command, versions/API/ABI, sanitized identifiers/timing and artifact paths. Nonzero on failure; a blocked prerequisite is separately recorded and cannot be counted as success. Runtime runner must not silently skip a selected backend. Unit fixtures may use deterministic provider transcripts/fault injection; live gates require actual providers.

## Isolated smoke ownership

Each run records UUID S plus exclusive short random suffix s. Use a private root `<workspace_root>/.gs-<s>` for project/integration/test state, where `<workspace_root>` is the deployment's configured wrapper-accessible workspace directory (for example `~/Workspace`). Exclusively create same-UID 0700 `$XDG_RUNTIME_DIR/gs-<s>/` and use its `admin.sock`; owned Herdr sockets also live there (or in source-supported canonical `~/.config/herdr` placement). Launch `goatr serve --admin-socket "$XDG_RUNTIME_DIR/gs-<s>/admin.sock"` directly, not through setup/production service installation. Pass that identical `--admin-socket PATH` to **every** `goatr pair`, `goatr devices` and `goatr revoke DEVICE_ID` invocation, including fault/restart/cleanup steps; no omitted override or fallback may contact production. **Never replace XDG_RUNTIME_DIR**: preserve the deployment's actual user runtime directory (normally `/run/user/<uid>`) and verify wrapper access. Derive host/provider-visible integration pairs through config; use `c.sock` and short relay socket basenames, preflight all socket UTF-8 lengths≤107 before effects. Herdr name `goatr-smoke-S`, unit prefix `goatr-smoke-S-`, owner unit `goatr-smoke-S-codex.service`; production paths/units never reused.

Transport-only modes run real packaged companion and second real iroh client; locally pair, hello, host.snapshot and authenticated notifications.read/sync where an event exists, or self-unpair for an empty host. **pair→hello→host.snapshot→device.unpair is the minimum successful framed authorization/action exchange**, not a provider control claim. They do not require credentials or create agents. Provider modes separately create an owned named Herdr server/project, actual desktop OMP/Codex wrappers and tiny prompted turns. Inject an owned already-bound loopback relay port into OMP runtime and overlay before launch; production remains7466. Never use a live production app-server merely because one exists.

Before cleanup, persist exact created endpoint identities, units, Linux boot/PID-start/socket inodes, Herdr name/socket, admin socket/private-parent path and ownership, runtime/config/state paths and emulator ownership in the run manifest. Cleanup targets only those still-matching owned resources; never wildcard-kill or delete pre-existing state. Service failures leave safe diagnostic evidence, not unrecorded leaks. Missing credentials/signing/ADB/host broker/user-service/public relay/ntfy prerequisites are named blocked gates; do not copy credentials, weaken wrappers/TLS, or substitute mocks.

## Required behavior checks

Each row below names a runnable owner from the command suite and observable evidence. The suite must implement these test modules/classes; paths are proposed, not assertions they exist.

### Protocol fixture parity

`companion/tests/test_protocol.py`, Kotlin `ProtocolFixtureTest`, shared `protocol/fixtures/`:
- Every method/result/error/event/type union, malformed/unknown behavior, framed Unicode/partial reads, recursive duplicate keys, surrogate/byte/member/depth bounds, UUID/Uint/timestamp and decimal-string rules. Include negative-zero/exponent/noninteger rejection and boolean-as-int.
- Shared canonical bytes/SHA256 and fixed-key HMAC vectors; method/default/field-order/array-order changes; exact answer-choice membership; no raw numbers leaking from native MCP forms.
- Auth/hello/receipt lookup/stale boot/target/dispatch check order; same-ID/same-hash lookup produces zero writes, changed payload conflict, old-boot status recovery, peer isolation and all timeout outcomes.
- Snapshot response precedes seq1; duplicate/gap/overflow resync, reconnect replacement, bounded history/content/page/inventory/form capacity and error mapping. No silent dropped request/history transition.
- `test_pairing.py`/`PairingTest`: exact admin NDJSON/credentials/private socket bounds; identical `--admin-socket PATH` handling for serve/pair/devices/revoke, unchanged omitted-option production default, rejection of relative/oversized/symlink/wrong-owner/nonprivate paths and wrong peer UID on client/server, absent overridden serve without fallback or second identity owner; wrong/expired/reused token, response-loss→known hello, unknown hello rejection, attempt limits, duplicate connections, deviceName/token lengths, restart expiry, scan/paste malformed input. Assert **zero session reads/provider operations before authorization**.

### Revocation fence

`test_security.py` plus `smoke.py --backend ...` fault points:
- Pause immediately before provider write, every creation stage and push attempt; revoke then release: no subsequent native effect/publish. Already-started bounded write may settle, desktop work remains alive; stuck boundary yields uncertain and cannot resume later.
- Admin completion waits for earlier write boundaries; self-unpair emits only its final response then closes; queued subscriptions/receipts/notifications inaccessible. Peer A cannot query B receipts/read-watermarks.
- Full host authority: two supported sessions inside/outside configured project list both appear and allow history/control/notifications for an authorized peer, neither for unknown peer; new creation outside configured projects rejected.

### Native libraries

`nix build`, `:iroh-jvm:test`, `NativeLoadTest` instrumentation, `check-native-libs.py`:
- Artifact manifest proves one FFI source/patch/lock across Kotlin/Python, full generated API compiles, resolved graph excludes GHSA-7cq4-mhxw-xw78/GHSA-jx4g-cg2x-jc35 and rustls meets pin. Record dependency/advisory snapshot, not version-name inference.
- Inspect APK ABI/native inventory, JNA AAR exclusion of desktop natives/JAR, Rust and JNA ELF/APK16-KiB alignment. Load both libraries and Android JNI context on owned emulator.
- Document API/ABI/device page size. Static alignment is not proof of running on a16-KiB device; unavailable real16-KiB runtime stays explicitly unexercised. Record copied-source notices and reproducible build input/license checks.

### Direct and relay transport

`smoke.py --transport direct|relay`:
- Direct uses endpoints without relays and explicit local address hints, observes selected direct path. Relay uses public N0 plus patched clear_ip_transports on **both** endpoints, observes selected relay path and completes identical framed authorized exchange.
- Source reference is core1.3.0 `endpoint_two_relay_only_no_ip`; no firewall changes/insecure TLS. Record actual selected-path events, not configured intent. Relay outage is blocked.
- Network callbacks/path watches/streams release without leaks; endpoint recreation retains identity; duplicate phone connection policy has one active subscription set.

### Provider source parity

`test_omp.py` and `test_codex.py`, shared sanitized pinned-source fixtures:
- OMP registry privacy/bounds/races/duplicate processes; link normalization (including nested fragments/loopback), AES envelopes, hello/welcome/final snapshot, persisted-versus-transient boundary, rich-file versus placeholder merge, optional title slot/partial tail/replacement/truncation, unknown record handling, select/editor/cancel and room-recreation dialog loss.
- OMP loopback relay wrong path/role/Host/Origin, nonupgrade, duplicate-host ownership, guest IDs/routing/room close, frame/queue overflow and configured test URL parity.
- Codex every finite method/decision/question/MCP field type/constraint/default/native answer map, integer/string native IDs, offered grants only, session/policy labels, secret exclusion, unsupported forms/userVerification. Command fixtures must include the pinned **experimental runtime** fields: present restricted/reordered `availableDecisions` with exact amendment payloads, rejection of any unoffered base/amendment answer, empty list with no invented choices, malformed/unknown decisions→desktop-only, and absent-list defaults for network context (first allow proposal only), additional permissions, and plain/execpolicy requests in exact source order. Exercise precedence when fields coexist; never substitute defaults for a present list. Preserve full `additionalPermissions` and scope or make the whole request desktop-only for unsupported/oversized details. Include malformed/oversized fields and browser-open-not-resolved.
- `test_codex_relay.py`: atomic manifest validation, socket/path/privacy bounds, owner/PID reuse, ready without consuming TUI slot, initial timeout, connected/disconnected/terminal_closed, bounded pending correlations, successful unsubscribe, unsolicited thread/started ignored, companion-independent helper/owner lifetime.
- Stable generated schemas omit experimental command fields and cannot alone define these source-parity fixtures; use the pinned runtime params, producer, decision conversion and legacy-default implementation linked in [References](REFERENCES.md#critical-files--anchors). Source fixtures still do not prove stock behavior: live provider gate separately checks loaded-list pagination, resume notification ordering, replay/first-response, native history pagination and busy turn/start.

### Live session and creation

`test_config.py`, `test_herdr.py`, `smoke.py --backend omp --backend codex`:
- Config schema/defaults/unknown keys/path ownership, project canonicalization and wrapper access, managed/symlinked conflicts, repeat setup preserving identity, optional service decline→foreground serve; one unavailable backend preserves the other. Owned smoke records unchanged XDG_RUNTIME_DIR and the same owned `--admin-socket` argument on serve and every admin command, proves pair/list/revoke and companion restart target only that socket, and leaves production admin socket/state untouched. No remote/TOML admin-path configuration or setup override is required.
- Record actual Herdr shell resolution to the installed wrapper; no fictional executable launch arg or unverified env.PATH guarantee. Fail creation preflight if proof missing. Prove namespace path pairs reference the intended object and sockets satisfy byte bounds.
- For each provider record owner PID/start, native conversation, Herdr incarnation/terminal, final binding and desktop geometry/focus before/after. Desktop and mobile prompts appear in the same native session without another engine/TUI or focus theft.
- No-focus creation returns ready target only after exact desktop binding. Unknown stage outcomes expose resources; exercise crashes before/after each side effect/result persistence, including workspace/thread/relay/TUI. Never second thread/launch, silent rollback or cleanup of user's work.
- Existing ambiguous/uninstrumented panes produce non-mutable UnboundRow; duplicate native IDs across owners, server restart/PID reuse and routing movement are distinct cases.

### Provider dialogs and cancellation

`test_omp.py`/`test_codex.py`, `ProviderFormsTest` instrumentation, real-provider smoke:
- Native approval/questions resolved from each side, simultaneous/late duplicate response closes once; stale key cannot affect newer request. OMP sequential select/editor/cancel, current-work Stop and desktop-only custom dialogs.
- Codex stale-turn cancellation rejected, offered session/policy grants exact, permissions scope explicit, secret input never stored, MCP standard types and explicit URL flow. Real first-response outcome is honest dispatched/unknown, never falsely this-phone-won.

### Reconnect and generation replacement

`test_state.py`, `SessionRecoveryTest`, provider/UI smoke:
- Join mid-turn/pending request; native reconnect replaces generation and final snapshot precedes queued events. Owner/thread/terminal replacement changes sessionId; room replacement/reconnect keeps proven sessionId but replaces generation; pane routing alone does neither.
- Companion boot invalidates all prior mutation contexts/handles. Old draft/request/content/cursor cannot retarget; file-history/newer-live precedence holds under racing pagination. Lost OMP room requests remain desktop-only, never replayed.

### Mutation receipts and companion restart

`test_receipts.py`, `test_creation.py`, provider crash-point runner:
- Fault every reserved→dispatching→native-write→receipt commit window, including pre-write crash. Drop reply and restart; action.status yields recorded outcome/uncertain, never automatic duplicate.
- Only definitive no-effect is failed; ambiguous write or unknown create ID uncertain; OMP no-ack remains dispatched. Same-ID lookup after stale boot is harmless, changed payload conflicts, unknown/expired receipts never authorize resend.
- Capacity/30-day retention, secret original/answer absent from SQLite/logs/drafts, keyed fingerprint fixtures, pending120-second create response and later action event/status. Revocation stops subsequent stages not already-started work. Companion restart leaves desktop owner/TUI/Herdr running.

### Content, cache and privacy

`test_content.py`, `test_notifications.py`, Kotlin `SessionStoreTest`/`RecoveryDraftTest`:
- Exercise **every bound/eviction row** in protocol with boundary and +1 values; multibyte byte offsets, TTL/LRU/generation expiry, source truncation, cursor invalidation, oversize form desktop-only, cache stale/partial markers.
- Offline required cached history, rotation and process-death recovery, local pre-send receipt does not erase draft, no retarget/automatic resend, protected-key loss fails closed. Secret answers/original pending JSON never enter persisted metadata or diagnostics.
- Offline Forget removes all host-local private/cache/push/dedup state without claiming server revoke; Android backup/transfer exclusions cover all databases/files/keys. Attention sequence survives restart/rotation, watermark per-session/per-peer, retention gap reset, live/push dedup and replay suppression.

### Compose behavior

Execute **`:app:connectedDebugAndroidTest`**, not merely compileAndroidTest. `NavigationTest`, `ProviderFormsTest`, `SessionRecoveryTest`, `PairingUiTest`, `NotificationEntryTest` use actual Compose runtime/semantics/testTagsAsResourceId:
- Narrow/landscape/IME/multiple forms/font scaling/TalkBack traversal, session selection/create/partial states, host-wide authority disclosure, no-project/ambiguous/setup/offline states, native Markdown/code backing-text copy/tool expansion/content refresh.
- All finite forms, option validation/scope labels/secret lifecycle, reconnect disablement, scanner denied/no-camera/Back/paste races and explicit-link safety. No source-text assertions/mock echoes standing in for UI.

### Emulator and camera evidence

`smoke.py --ui --serial ...` owns resources, installs/launches real APK and sends/stops through visible UI against real companion/providers. Save fresh PNGs for pairing, all-branches OMP conversation/tool cards, Codex conversation/forms, partial/uncertain creation, cold/warm notification entry and offline cache. Correlate screenshot timestamp with commands and source/native identities.

`QrDecoderTest` covers generated QR→padded/rotated camera planes and frame-finally closure; scanner instrumentation exercises lifecycle/cancellation and posted-callback disposal. Report actual camera input separately: paste and decoder fixtures do not prove camera hardware input. Never use a physical phone without authorization.

### Background delivery

`test_push.py`/`test_notifications.py`, `PushStateTest`, explicit `smoke.py --background persistent|ntfy --serial ...`:
- Persist-before-register; probe before response; process death before/after persistence; duplicate probe/ACK/ACK-response loss; deadline restart; stale unregister/ACK/generation; endpoint rotation and old-key rejection; no handoff before confirmed ACK.
- Event/probe payload alternatives (probe has no session), HMAC/base64/identity/size validation, same-ID live/push/retry dedup, durable sequence/read reconciliation, source completion versus disconnect, peer scoping/retention gap.
- Approved-origin HTTPS validation, query preservation, redirect/DNS rebinding/private-target denial, TLS error/no insecure bypass, HTTP429/5xx/404/410/auth responses, retry horizon/queue bounds and revocation cancellation. Deterministic HTTP fixtures prove failure policy; real ntfy proves distributor delivery.
- Real persistent attention and ntfy probe→handoff→attention→click→current-state flow, endpoint recreation/network changes with retained identity, permission denial/force-stop/Doze/service restrictions and visible repair without silent mode changes. Record API level and externally induced limits; never claim these platform/network limits eliminated.
