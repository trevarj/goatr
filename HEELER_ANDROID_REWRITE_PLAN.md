# Heeler Android rewrite: studio-first review gate

## Context
The user reports that the current Goatr application is unacceptable and non-functioning and requests a complete Android rewrite matching Heeler's visuals and behavior, except for underlying libraries and explicitly selected platform/transport differences. Research and several question rounds established the target below. The user then explicitly selected **a localhost web studio containing reference screenshots and annotated wireframes, delivered for their review before any production UI rewrite**. This approval therefore executes the studio/rewrite-contract gate only; the full application remains required, but production implementation must wait for that review and a subsequent implementation handoff, not be presented as completed by this studio.

## Settled rewrite contract
These decisions must appear in the studio and survive the later implementation handoff without reopening settled preferences:

|Area|Decision|
|---|---|
|Reference|Freeze `ZingerLittleBee/Heeler` at `17e890481ae5a80e142395ab17974cee8655c6a4`, including its current-main herdr-core feature scope. Do not follow moving main or substitute v0.1.17.|
|Fidelity|Reproduce screen composition, density, controls and behavior, not a Material redesign. Adapt OS-owned keyboard, permissions, notifications/widgets and window handling to Android. Preserve Goatr's name and identity.|
|UI|Kotlin with native Android Views, platform text/input/list facilities and custom layout/drawing where necessary. No Compose application framework; no wholesale import of Telegram's application framework.|
|Android|Minimum API 33 (Android 13). Phones, tablets and foldables, adaptive panes, resizing, hardware keyboards and pointer behavior. The old Mi 9T/API-30 phone is not an acceptance target.|
|Host|Linux only; both Herdr TUI and herdr-core are required where the reference supports them. Official stock runtime upgrades are allowed; private runtime patches are not. No global installation or running-service replacement is authorized by this studio gate.|
|Connectivity|Retain iroh and n0 public relays, direct connections when available, encrypted relay fallback. Users install a companion and pair; they do not configure SSH/VPN or host a relay. Public relay rate limits and lack of SLA are accepted, not disguised as production service qualification.|
|Pairing|Keep the genuine scan/paste → identity/trust preview → explicit confirmation experience, using iroh identities rather than SSH enrollment. Never embed a managed-service project secret.|
|Terminal authority|Real interactive agent terminals, resize and takeover; explicit confirmed Close/worktree removal. Explain actual native scope and desktop effects, including final-pane/workspace cascades. Old read-only/no-Close restrictions are superseded.|
|Notifications|Managed ntfy plus a UnifiedPush distributor app. End-to-end encrypt payloads; authenticated-only messages are insufficient. No Goatr FCM sender service.|
|Migration|Fresh pairing and a clean protocol/data cutover are acceptable. Preserve existing source and installation data until explicitly replacing them; do not spend effort on compatibility/migration layers.|
|Reuse|Qualify and reuse sound existing implementations. User has no licensing preference; comply with actual file/revision licenses and notices, without asking them to pick a license.|
|Evidence|Only public Heeler source/assets are available. Owned Android emulators are reauthorized and may suffice for completion. Do not claim observed iOS runtime parity or physical-phone performance/battery/background behavior from source or emulator evidence.|
|Provider calls|Later acceptance may use existing configured accounts for small bounded prompts. No new signup/subscription or credential collection. This studio gate makes no provider calls.|
|Review order|Studio first, then user review. The first subsequent working application checkpoint is genuine pairing → live console → real terminal input/reconnect. User reviews matched captures at checkpoints before the UI expands.|

Transport-specific SSH enrollment, jump-host and overlay setup are implementation differences to disclose in the studio, not fake controls leading nowhere. Preserve the user outcomes—multiple hosts, remote reachability, route health and identity verification—through the selected iroh architecture. macOS/Windows support is an explicit exclusion selected by the user. Android system surfaces are adaptations, not claims to reproduce Dynamic Island or every vendor's lock-screen widgets literally.

## Grounded starting point
- Working directory: `/home/trev/Workspace/goatr`. The checkout has `app/`, `companion/`, `iroh-jvm/`, `iroh-android/`, `protocol/`, `third_party/iroh-ffi/`, `flake.nix` and `Makefile`. No existing web studio was found in the root/source investigation.
- `docs/` is empty. `MOBILE_HERDR_MVP_PLAN.md:24` explicitly records its intentional deletion. Do not restore it. The retained plan's Orca-inspired, read-only-agent, no-Close, old-peer-compatibility scope is historical and conflicts with this rewrite.
- Android currently uses Compose, an application-owned `GoatrRuntime`, `GoatrConnection` publishing `StateFlow`, `SessionStore` backed by SQLite schema 8, AndroidKeyStore-backed `SecretStore`, and native editor/camera/text Views embedded through Compose. Preserve useful behavior only after qualification; do not assume presentation/state are already separated.
- Current `TerminalGrid`/Rust observer code receives projected cells, not a full raw terminal stream. Replacing its Canvas alone cannot deliver Heeler terminal parity.
- `companion/Cargo.toml` is one Rust package using iroh 1.3.0, Tokio, serde, SQLite and AES-GCM. `Protocol.kt` and `transport.rs` already implement bounded length-prefixed frames, identity checks and no-replay/recovery behavior. No transport or protocol cutover is implemented in this studio step.
- `flake.nix:56-66` owns development tools. The retained nixpkgs source includes `pkgs.darkhttpd` 1.17 (`pkgs/by-name/da/darkhttpd/package.nix` was read). `Makefile` has no studio target. There is no need for a custom HTTP server, React, Vite, npm workspace, database or browser-hosted terminal.
- All research was static. No app, existing client's APK, runtime upgrade, emulator, provider call or transport was exercised during planning. The user's report is accepted; do not rerun the broken application merely to confirm it.

## Approach

### 1. Preserve the checkout and establish the bounded studio ownership
The coordinator is integration owner. Use one implementation writer, with no recursive delegation and no concurrent production edits. Its lease is new `studio/`, the `tools` entry in `flake.nix`, and the studio target in `Makefile`. Existing Android, companion, protocol, service/configuration and runtime files are read-only during this gate.

Before mutation, verify the absolute cwd, inspect the existing worktree/index state once for preservation, and retain private preimages/hashes of any existing untracked file that actually needs editing. Preserve unrelated changes, intentional deletions, existing caches and installed applications. Do not commit, push, install globally, launch a device, replace a service or delete old application code.

Create these files only as needed for the fully working studio:
- `studio/index.html`: semantic shell, navigation, comparison workspace and provenance/status panels.
- `studio/studio.css`: responsive review layout and reference-derived wireframe tokens.
- `studio/studio.js`: catalog loading/validation, selection/filtering, image comparison and local review notes.
- `studio/wireframes.js`: static SVG scene rendering using shared small drawing helpers and scene-specific layouts; not a simulated application or generic screen-builder framework.
- `studio/catalog.json`: sole feature/screen/reference/decision/milestone inventory, with the concrete shape below.
- `studio/checks.js`: small browser-runnable behavioral checks for catalog validation, comparison boundaries and evidence classification.
- `studio/reference/heeler/`: only the selected public images and upstream license/notices needed for those assets; no whole-source checkout or private data.
- `studio/vendor/pixelmatch.js` and its `LICENSE`: exact pinned comparison implementation.

Use existing platform HTML controls for search, selects, tabs, ranges and file selection. Keep behavior in the listed modules; do not create a routing/state/component framework. External source links are citations, not runtime dependencies.

### 2. Freeze source assets and populate a complete parity catalog
Use `https://raw.githubusercontent.com/ZingerLittleBee/Heeler/17e890481ae5a80e142395ab17974cee8655c6a4/` as the asset/source base. Retain original downloaded image bytes and record SHA-256, actual image format and dimensions; choose the local extension from the decoded format rather than assuming every upstream `.png` contains PNG. The reader returned WebP for inspected `chat-iphone.png` and `console-ipad.png`. Do not re-encode reference images or hotlink them into the running studio.

The pinned README establishes these 12 phone assets under `docs/images/`:
`console-iphone.png`, `live-terminal-iphone.png`, `chat-iphone.png`, `agent-iphone.png`, `changes-iphone.png`, `terminal-iphone.png`, `skills-iphone.png`, `hosts-iphone.png`, `live-activity-iphone.png`, `usage-iphone.png`, `widgets-iphone.png`, `widgets-lock-iphone.png`.

The pinned `docs/ipad-screenshots.md` establishes these seven tablet assets:
`console-ipad.png`, `terminal-keyboard-ipad.png`, `skills-ipad.png`, `terminal-ipad.png`, `changes-ipad.png`, `diff-ipad.png`, `windowed-ipad.png`.

The inspected chat image was 1320×2868 and tablet console was 2752×2064. Re-measure fetched bytes; these are raster sizes, not proof of logical iOS point dimensions. Associate screenshots with their pinned source files, but do not assume an image proves a feature introduced later than its capture.

Vendor Pixelmatch **8.0.0**, commit `bddb61314b2db3ad7e6f1ec57b38bf2dbae81184`, from its `index.js` and `LICENSE`. Its browser entry is the default export `pixelmatch(img1, img2, output, width, height, options)`. Keep its ISC notice. No CDN or npm installation is needed.

`catalog.json` uses this concrete model (arrays contain full records, not placeholder entries):
- `schemaVersion: 1`.
- `designRevision: 1`, a positive integer incremented whenever wireframe geometry/content, variant definitions, behavior annotations or associated references change. This identifies the reviewed design independently of the upstream reference pin.
- `reference: { repository, revision }`, with the frozen Heeler repository URL and commit above.
- `decisions: [{ id, title, value }]`, populated from the settled contract.
- `assets: [{ id, kind, path, mime, width, height, sha256, sourceUrl, viewport, theme }]`. Initial `kind` is `reference`; future genuine captures may use `fixture_capture`, `emulator_capture` or `physical_capture`. `sourceUrl` is required for upstream assets. Asset `viewport` and `theme` describe the actual capture, with `unknown` allowed when not established. Paths must be relative within `studio/`; no absolute paths, `..`, URL schemes or symlink escapes.
- `screens: [{ id, title, group, sourceUrls, featureIds, variants }]`. Each variant is `{ id, viewport, theme, referenceAssetIds, annotations, evidenceNote }`; `viewport` is `phone`, `tablet`, `folded`, or `book`; theme is `dark` or `light`. Reference IDs belong to this exact variant, not the screen globally. An empty reference list is valid and requires an evidence note explaining the source-derived adaptation. A related screenshot from another theme/form factor may be shown only with that mismatch stated in `evidenceNote` and in its visible caption. Each annotation is `{ text, basis }`, with basis `screenshot`, `source`, or `android_adaptation`. Wireframes are generated for a selected screen/variant and always labeled `wireframe`; they are never stored as genuine captures.
- `features: [{ id, title, screenIds, backends, disposition, behavior, acceptance, sourceUrls }]`. `backends` uses `tui`, `core`, `device` as applicable. `disposition` is `required`, `adapted`, `reference_unavailable`, or `excluded`. `acceptance` is `{ given, when, then }` describing an observable scenario. There is no implementation-pass field at this gate.
- `milestones: [{ id, title, featureIds, gate }]`, populated from the later checkpoint sequence below.
- `research: [{ id, title, sourceUrl, finding, qualification }]`, covering the Android clients, Telegram, transport alternatives and validation tools already investigated.

Validate unique IDs, enums, all cross-references, relative asset paths, required provenance and nonempty acceptance steps before rendering. On invalid catalog, show `Catalog unavailable` and the exact offending record/field; do not quietly drop rows or show an empty-success dashboard. Empty search results show `No matching screens`. Missing/undecodable assets show `Reference unavailable` with the pinned source link; never use a generated substitute labeled as a screenshot.

Keep the module boundary concrete: `studio.js` exports `validateCatalog(value)` (returns the validated catalog or throws a record/field error) and `compareRgba(left, right)` (two `ImageData` inputs, returning `{ differentPixels, diff }`, with `diff` an `ImageData`; throws on unequal dimensions). `wireframes.js` exports `renderWireframe(screen, variant)` returning an `SVGSVGElement`. `checks.js` exports async `run()` returning named passing results and throwing on the first failed check. Catalog and comparison code must be the same functions used by the visible studio, not independent test-only copies.

Each meaningful capability must have a feature row, not just one row per marketing heading. Populate at least the following groups and their listed behaviors:

|Group|Required inventory and source anchor|
|---|---|
|Console/navigation|Multi-host status ordering; host filter/search; pins; flat/by-host/by-workspace groups; linked worktrees/collapse state; configurable row fields; agent switching; edge drawer; remembered Chat/Terminal choice; Terminal default; deep links. `Sources/Heeler/Console/ConsoleView.swift`, `CONTEXT.md`, `CHANGELOG.md`.|
|Terminal|Raw attach stream; full styles/Unicode; scrollback; alternate-screen and mouse scrolling; momentum; local composer versus Direct Input; full keyboard/modifiers; selection sheet; links; themes/fonts/zoom; retained engine; reconnect/takeover; truthful uncertain input; Kitty graphics including layers/placeholders/animation. `TerminalScreenView.swift`, `TerminalTextSelectionPresenter.swift`, `AttachLinkIndex.swift`, `TerminalMouseReporting.swift`. Sixel is reference-unavailable, not an extra implementation task.|
|Chat|Claude Code, Codex, Pi, OMP, Grok Build and OpenCode on TUI; five on Linux core, with OpenCode marked `reference_unavailable` there. Rich messages/Markdown/tables, grouped tools, turn folding, history/paging/compaction, images, offline cache, explicit conversation choice, guarded Blocked controls, unknown outcome without replay. `Chat/Conversation/ChatTranscriptAdapter+Programs.swift`, `Chat/Source/HerdrCoreSessionBinder.swift` and program-specific reducers.|
|Composer/tools|Drafts retained across modes/agent changes, Send/Stop distinctions, keyboard shortcuts, Skills, snippets, attachments, voice. Insertion never silently submits. Confirmed source directories under `Sources/Heeler/`: `Skills/`, `Snippets/`, `Attachments/`, `VoiceInput/`; use the inspected `VoiceInputEndpoint`, `VoiceTranscriptionClient`, `VoiceInputSession` and `VoiceRecorder` definitions.|
|Shells/creation|Existing shells, workspace cards, Text/Keys modes, reuse versus explicit create, program/directory discovery, workspaces and worktrees, launch arguments, account environment, device-local Launch Presets and native core Templates. `Transport/Transport.swift`, `Console/HerdrCoreConsoleProjection.swift`, `Accounts/LaunchPreset.swift`.|
|Management/Changes|Rename where native support exists; confirmed actual Close; worktree risks/removal/partial outcomes; staged/unstaged/untracked files, branch/upstream counts; unified/side-by-side word-highlighted diffs. Do not add an editor or commit/stage/reset workflow. `Changes/GitProbe.swift` and extensions, `Console/HerdrCoreWorktrees.swift`.|
|Hosts/pairing|Multi-host inventory, backend/session selection, health/reconnect/remove, scan/paste/preview/Confirm, identity mismatch/revocation; replace SSH-specific enrollment/routes with the selected companion/iroh experience and disclose this adaptation. Never generate a real pairing token for the studio.|
|Accounts/config|Host account directories and variables; add/rename/remove/ignore; provider configuration preview/explicit confirmation/undo; helper/service health. Accounts are host files, not a fabricated core API. `Accounts/HostAccountsStore.swift`, `HarnessConfigEditStore`, `HostServices.swift`.|
|Notifications/system UI|Per-host Blocked/Done and core Error notifications, encryption, preference/permission/distributor health, foreground suppression, deep-link identity checks, per-device seen/Done behavior, ongoing status and widgets as Android adaptations. No APNs relay reuse.|
|Usage/appearance|Claude/Codex limits/reset times, 30-day token/cost views and filters/breakdowns, global off switch, widgets; theme modes, 30 terminal themes, light/dark slots, JetBrains Mono/IBM Plex Mono, scale. Claude's undocumented usage query stays off by default as in the reference.|
|Adaptive layouts|Phone, landscape/resizing, tablet sidebar/detail, foldable separation, hardware keyboard/pointer, drag/drop and multiwindow state. Android system differences are explicitly annotated.|

Use precise pinned file URLs for every row; investigate the specific source if a behavior is unclear, and label source-only behavior rather than inventing a screenshot observation. The research above is a starting map, not permission to omit feature rows found in the pinned source. Features intentionally unavailable in the reference need their own explanation, not a false failed port or invented implementation.

### 3. Build annotated wireframes rather than a simulated app
Create source-specific static SVG scenes in `wireframes.js`. Use shared drawing helpers only for repeated geometry/text; the compositions and annotations must differ by actual screen. No clickable Send/Close/pairing actions or pretend live hosts. The studio controls select scenes; controls inside the depicted app are illustrations.

Use deterministic non-sensitive sample labels (`Studio Linux`, `Build Linux`, `Payments API`, `Storefront`, `Docs`, `/home/demo/work/project`) and the source's status vocabulary. Pairing displays `DEMO — NOT A PAIRING CODE`, never a syntactically valid invitation, secret-looking token or scannable QR payload.

Required scene IDs and content:

|Scene IDs|Required composition/state|
|---|---|
|`pair-entry`, `pair-preview`, `pair-error`|Scan/Paste entry; identity/trust confirmation; expiry or mismatch refusal. Adapt enrollment, not the trust ceremony.|
|`console`, `console-grouped`, `console-search`, `console-empty`, `console-disconnected`, `console-row-layout`|Reference-density rows, status/pins/host context; grouping/search; clear empty/stale states; field customization.|
|`agent-terminal`, `terminal-keyboard`, `terminal-selection-links`|Reference header, full terminal region, status/composer/dock; Direct Input/tools keyboard; native-style selection sheet and collected HTTP(S) links.|
|`agent-chat`, `chat-blocked`, `chat-history-offline`|Rich response with table/tool groups, user message, composer/dock; approval/question card with explicit command and caution; paged/offline conversation.|
|`composer-tools`, `skills`, `snippets`, `attachments`, `voice`|Distinct panel contents; insertion versus Send; staging progress/error; recording/consent/cancel/retry.|
|`shells`, `launch-agent`, `launch-terminal`, `worktrees`|Shell inventory and text/key modes; actual launch forms/directory/worktree choices; destructive preview including partial-failure information.|
|`changes`, `diff`|File status/line counts and checkout metadata; unified phone diff and side-by-side tablet diff with word highlights.|
|`hosts`, `host-detail`, `accounts`, `launch-presets`, `core-templates`|Backend health, pairing/removal/reconnect, account directories, preset args/env and native template distinction.|
|`usage`, `notification-settings`, `ongoing-notification`, `widgets`|Usage limits/charts; distributor/permission health; Android ongoing/attention notification; home-screen widget equivalents. Explain unsupported OS-specific equivalents rather than inventing portable lock-screen widgets.|
|`appearance`, `settings`|Theme/font/light-dark slots, display scale and remaining actual settings groups.|

All scenes require a dark phone variant. Add light phone variants for `console`, `agent-terminal`, `agent-chat`, `hosts`, `usage`, `appearance`. Add dark tablet variants for `console`, `agent-terminal`, `terminal-keyboard`, `agent-chat`, `chat-blocked`, `skills`, `shells`, `changes`, `diff`, `usage`. Add both dark `folded` and dark `book` variants for `console`, `agent-chat`, `agent-terminal`.

Design canvases are phone **412×915**, tablet **1280×800**, folded/compact-cover **360×800**, and open-book **840×900** CSS units. The open-book design has a vertical full-height 12-unit hinge at `x=414`, yielding safe panes `[0,414)×[0,900)` and `[426,840)×[0,900)`. Show directory/agent navigation in the left pane and the selected console detail, chat or terminal in the right pane; no text, controls or composer cross the hinge. The compact-cover variant has no hinge inside its active screen. These are chosen Android design geometries, not measured iOS layouts or tested physical devices. Reserve Android system inset/keyboard areas explicitly; do not stretch a phone image into a tablet or book layout. Insets, 48dp minimum touch-target intent, focus/accessibility labels and enlarged-text behavior are annotations where a static scene cannot prove them. Wireframe text/fonts are schematic and never labeled pixel-perfect final typography.

For each scene show 3–6 concrete annotations: reference-derived geometry/ordering, key behavior, Android adaptation and any evidence limitation. Example: Chat table overflow is horizontally scrollable within its content area; it must not push the composer offscreen. Do not substitute generic cards or blank boxes for tables/diffs/terminal/keyboard layouts.

### 4. Implement the working review surface and comparisons
Desktop layout: searchable grouped screen catalog on the left, central reference/design comparison, right-hand annotations/feature scenarios/provenance. On narrow browsers stack these sections without clipping controls; the depicted Android canvas remains independently zoomable. Use keyboard-accessible HTML controls, visible focus, labeled comparison sliders and descriptive image alternatives.

Expose these real studio operations:
1. Select a screen, form factor and theme; show references associated with that exact variant and its wireframe. Preserve selection in the URL hash (`#screen=<id>&viewport=<phone|tablet|folded|book>&theme=<dark|light>`). Offer only variants that exist; invalid hashes fall back to `console/phone/dark` with a visible explanation, not arbitrary code/path loading. A source-derived light/foldable layout with no screenshot shows its evidence limitation rather than inheriting an unlabeled phone/dark reference.
2. Switch `Side by side`, `Overlay`, `Difference`. Side-by-side fits each image proportionally and shows intrinsic dimensions. Overlay/difference require equal decoded dimensions after a deliberately selected crop; do not silently resize/stretch either input. For unequal canvases show `Different canvases — side-by-side only` and disable numeric comparison. Stage-one wireframes generally compare visually, not numerically, with iOS screenshots.
3. Provide two local image pickers for comparing actual captures later. Accept PNG/JPEG/WebP only, max 16 MiB per file, max 4096 pixels per dimension and 16 million decoded pixels. Reject oversize/invalid input visibly; revoke object URLs and release superseded pixel buffers. Files stay in browser memory and are never uploaded or copied into the gallery automatically.
4. Use Pixelmatch with `threshold: 0.1`, `includeAA: false`; display differing-pixel count and input dimensions. There is **no universal parity pass percentage**. Cross-platform reference comparisons are diagnostic; precise regression gates belong to matched Android captures after design approval. Any user-chosen crop must be shown with coordinates; initial crop is the whole image. Cropping is a numeric x/y/width/height form clamped to the source image, not another custom gesture library.
5. Toggle annotations. All imported labels, notes and catalog text use text nodes/textContent, never interpolated executable HTML. Render locally authored SVG using DOM APIs; reject external href/script/foreignObject content.
6. Record design review state `unreviewed`, `accepted`, or `changes_requested`, plus free-text notes, separately for each `(screenId, variantId)`. Store `{schemaVersion:1, referenceRevision, designRevision, reviews:[{screenId, variantId, state, note}]}` in browser localStorage under `goatr.studio.review.v1.<reference-revision>`, and export that same shape as a JSON download. On a design-revision mismatch retain previous notes and display the previous state as stale, but set effective review state to `unreviewed` with `Review needs renewal`; no acceptance carries forward automatically. If storage is unavailable/full, continue in memory with `Notes are not persisted` visible. Do not add import, backend saving, accounts or a collaboration system. Design acceptance never changes a feature's implementation status or approves unreviewed variants.
7. Show a permanent banner `Design review — application rewrite not implemented`. Display artifact badges `Reference`, `Wireframe`, `Fixture capture`, `Emulator capture`, `Physical capture` separately; reference/source-derived intent must never be presented as runtime proof.

Use a restrictive meta CSP allowing only local scripts/styles/assets and browser-local image blobs, no external fetches or embedded remote frames. Cite sources with normal external links opened with `noopener noreferrer`. Once assets are present, the studio must work with upstream network access blocked. Do not use service workers or a public deployment.

### 5. Put the actual rewrite architecture and acceptance sequence in the studio
This is required review content, not permission to start production implementation before the user review. Show the following concrete choices and evidence alongside the feature catalog so the studio is not just an attractive mood board.

**Architecture to carry into the later implementation plan:**
- One application-owned runtime/connection/store, preserving useful `GoatrRuntime` ownership and `StateFlow` behavior; Views observe it rather than creating a second endpoint/store. Requalify `SecretStore`, genuine pairing, native IME, no-replay/recovery and strict framing. Compose-specific state must be removed together with all callers, not just its Gradle plugin.
- Linux Rust companion performs local Unix-socket, PTY, filesystem, bounded process-evidence and Git operations that Heeler performs through SSH. No arbitrary remote-shell endpoint or new SSH/VPN setup is needed. Keep native backend distinctions.
- TUI uses `session.snapshot`, `events.subscribe`, real native launch/control/Close methods and a local PTY running official `herdr agent attach <pane-id> [--takeover]` or `herdr terminal attach <terminal-id> [--takeover]`. Events are subscribed before the initial snapshot. Retain attach bootstrap-query handling and input uncertainty.
- Core uses JSON-RPC `hello {generation:1, framed:true}` and required `framed/attach/events/status` capabilities, then its framed channels/credit and `terminal.attach`, `terminal.run`, `terminal.interrupt`, `terminal.kill`. Do not invent core `workspace.*`, `tab.*`, account or preset APIs. Read `herdr-app` organization/labels; only use actual supported desktop rename controls. A passive reconnect must not autostart a core.
- Structured Chat follows actual provider transcripts and process/session identity. TUI supports six reference providers; Linux core supports five, with OpenCode unavailable at this reference pin. Accounts are host-side JSON, Launch Presets are device-local and Templates are native core operations.
- Terminal engine is Ghostty **`82938b633ba646db38591d969c3c526332bd7e65`**, underlying Heeler's libghostty-spm **`7e45d27160f9b34aca9ca5c9820e9207482f9f04`**, built with Zig **0.16.0**. The Android build of this exact pin is unverified. Use a small JNI/Zig adapter and custom Android View, not Apple UIKit/Metal or the old grid→VT projection. Include complete styles, key encoders, IME, scrollback, native selection, links and Kitty images. Virtual placements and animation ticking require native Zig access not fully exposed by the C API; reuse `graphics_unicode.zig` placement iteration and `ImageStorage.animationTick`, not a Kotlin reimplementation. No Sixel claim: the reference engine does not implement it.
- No full client is a drop-in base. Bellterm's custom `TerminalView.kt` is a useful permissive View/IME/gesture donor, but its grid→VT/no-inline-image backend is not the chosen architecture. The older Android Heeler fork has useful JNI/graphics plumbing, but loses style/cursor/image-layer information and needs IME/UTF-8 corrections. Flutter herdr-mobile has useful workflow references but its documented terminal/provider limitations are not parity.
- ntfy delivery must use encryption, unlike current Goatr's HMAC-only payload. Port the reference's per-host/device privacy and seen/Done semantics, not its APNs service. Android ongoing notification/widgets are explicitly adapted system surfaces.

**Later checkpoint order to display (all remain required):**
1. `studio-review`: this complete studio and user feedback. No production rewrite before this gate.
2. `pair-console-terminal`: qualify official Linux runtimes, implement clean companion/app cutover, pair genuinely, discover hosts/agents, render/control real TUI and core terminals, reconnect without input replay. First real APK checkpoint and capture review.
3. `chat-composer`: provider transcript binding/rendering, Blocked actions, local drafts, tools/skills/snippets, attachments and voice, with rich content and keyboard interaction. User capture review before expanding UI.
4. `creation-management`: shells, launch/workspaces/worktrees, Changes/diffs, accounts/presets/templates, real Close and partial destructive outcomes. No pretend success or guessed atomicity.
5. `delivery-usage-adaptive`: encrypted ntfy/UnifiedPush, ongoing status/widgets, usage/limits, full themes and phone/tablet/foldable/window/keyboard behavior.
6. `parity-acceptance`: every required/adapted catalog row tied to actual scenario evidence; matched Android screenshots plus real host effects, direct/relay paths, interruption recovery, security/error cases and measured emulator performance. Unavailable provider credentials remain a named blocker, not fixture-passed live qualification.

Record these official-version research anchors without claiming qualification: Herdr TUI v0.9.5 `2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`; core v0.1.13 `88fe4e752debdc6ff6689eaa153d7f95819835a4` is the reference's recorded Linux baseline, while later core v0.1.18 `3965a46caae095a3ead01de237fa13955e52b8c6` is also recorded for other-platform checks. The next implementation handoff must freeze the official Linux runtime pair through capability qualification; the studio does not select a fake compatible version or install either.

**Validation contract to display:**
- Source fixtures and public screenshots establish intended behavior, not observed iOS execution.
- UI Automator drives the actual Android app/system UI; Macrobenchmark and Perfetto measure release-like scrolling, keyboard transitions, agent switching and terminal output on a recorded environment. Emulator results remain emulator results.
- Same-state, same-size Android captures become regression goldens only after user design approval. Cross-platform comparisons expose geometry/typography/control differences; they do not use an arbitrary whole-image percentage to declare parity.
- Live integration scenarios verify actual effects: pair the intended host, input reaches the intended terminal once, reconnect does not repeat it, a stale target cannot receive a queued action, Blocked choices apply only to the still-current prompt, Close acts only within its displayed native scope, and background notification opens the correct identity-bound target.
- Graphics checks include fragmented VT, Unicode/wide cells, complete styles/cursors, key press/repeat/release, CJK composition, bracketed paste, mouse/alternate-screen scrolling and Kitty layers/placeholders/animation. A native-library load or cell dump alone does not pass.

### 6. Serve only the curated studio using existing tooling
Add `pkgs.darkhttpd` to `flake.nix`'s existing `tools` list; do not update nixpkgs/rust-overlay inputs or introduce another flake. Add `studio` to Makefile `.PHONY` and a target executing:

```sh
darkhttpd studio --addr 127.0.0.1 --port 4173
```

Run from `/home/trev/Workspace/goatr` using:

```sh
nix develop path:. -c make studio
```

Serve `studio/` only, never the repository root, home directory or evidence tree. The gallery contains only public reference assets and explicitly safe design/demo content. Use a named foreground service in the agent tool; no daemon installation, global service or production port changes. If port 4173 is occupied, leave its owner untouched and run the same command on 4174, reporting the actual URL. If both are occupied, select the first free port from 4175–4183; never terminate an existing listener.

After browser verification, leave the studio available for the user's review using session service lifetime controls, report the URL and exactly what is ready, and pause at the requested review gate. Do not start removing Compose, replacing the companion, installing a runtime or launching provider sessions while waiting for this review. Use the user's review to produce the subsequent production implementation handoff; this studio is not completion of the full app request.

## Critical files & anchors
- `MOBILE_HERDR_MVP_PLAN.md:3-24`: old scope and intentional docs deletion; read for preservation, do not continue its superseded Orca/read-only acceptance path.
- `flake.nix:54-66`: existing development-tool owner; add the static server here only.
- `Makefile:1-39`: existing command conventions; add the studio entry without altering current checks/build behavior.
- `Sources/Heeler/Transport/HostServices.swift` at the frozen upstream commit: separates device/local-host operations from SSH wrappers and prevents invented runtime APIs in the catalog.
- `Sources/Heeler/Console/HerdrCoreConsoleProjection.swift` at the frozen commit: source for core-versus-TUI capability distinctions and adaptive console behavior.

## Verification
All executable checks below occur only after approval, from `/home/trev/Workspace/goatr`. This is a web/design gate: unrelated Android/Rust builds, devices and providers are deliberately not exercised here.

1. Start the exact `nix develop path:. -c make studio` command as a named service. Observe that only loopback is listening and the served root is `studio/`; request `/` and `/catalog.json` and verify actual content. A request for `/../companion/Cargo.toml` or `/companion/Cargo.toml` must not expose repository files. Use the server's actual URL if the prescribed port fallback was necessary.
2. Open the real page with the browser tool; read its browser documentation before use. Exercise search, screen/form-factor/theme selection, URL-hash restoration, annotations and feature/source views at desktop 1440×1000 and narrow 390×844 browser sizes. Verify all required phone/tablet/compact-cover/open-book scene variants render distinct source-derived content, with no horizontal control clipping, broken reference images or console errors. Verify open-book content avoids the defined hinge and missing light/foldable reference images are explicitly labeled. Capture the actual studio overview and phone/tablet/book detail views as review evidence, not app screenshots.
3. Check every catalog asset's file existence, SHA-256, decoded dimensions/format and source pin. Check all screen↔feature references and variant IDs, and confirm the 19 named reference images are present or explicitly reported as upstream-unavailable. If an image is unreachable, retry the pinned raw URL and the pinned GitHub blob/download route before classifying it; do not silently remove its scene.
4. Execute the permanent browser checks with `await import('/checks.js').then(m => m.run())` in the studio tab. Implement `run()` to throw on any failed assertion and return named results for: invalid/duplicate catalog IDs rejected; dangling feature/asset reference rejected; unsafe asset path rejected; equal RGBA images produce zero differences; one changed opaque pixel produces one difference; unequal dimensions are refused without resizing; `wireframe` cannot be classified as a genuine runtime capture; a variant with no screenshot retains its explicit evidence limitation; a design-revision change retains review notes but invalidates prior acceptance. These cover user-visible failures, not source text or copied constants.
5. Through the visible image pickers, compare a screenshot with itself, then a deliberately modified local copy: observe zero versus nonzero difference and visible changed-region highlighting. Use a differently sized image and observe `Different canvases — side-by-side only`. Verify oversized/invalid file errors leave the existing comparison usable.
6. Enter a review note containing `<img src=x onerror=alert(1)>`; it must remain literal text. Set `changes_requested`, reload and verify the review persists for the same reference/design/variant only. Export JSON and inspect the actual screen ID, variant ID, state, note and both revisions. Change the design revision in a test catalog, reload the saved review and observe retained notes plus `Review needs renewal`, not an accepted changed design. Simulate unavailable localStorage and verify the in-memory warning/fallback, not data silently claimed saved.
7. Block external network access in the browser after local assets are loaded; reload and exercise the studio. It must not request a CDN, analytics, remote screenshots, provider service or credentials. Source links may remain ordinary outbound links, but viewing the studio must not depend on them.
8. Verify zero product-pass claims: no source or wireframe marked as live-tested, no provider/notification/native-runtime checks shown passed, and the fixed design-review banner visible. Close browser tabs after evidence capture; keep only the named loopback server for the user review.

## Assumptions & contingencies
- The user explicitly requested this studio-first stop. Subsequent production code changes require the review handoff; silence is not approval to skip it.
- The chosen design canvases are review sizes, not a physical-device commitment. The user can request other sizes in the studio review; the full Android target remains API33+ phones/tablets/foldables.
- Public reference assets may not cover every source feature or light theme. Render the missing view as a source-derived wireframe with an explicit limitation; never fabricate a reference screenshot or claim an observed transition.
- If pinned source/assets are temporarily unreachable, retain the known pin and report the exact unavailable artifact after alternate-route attempts. Finish reachable studio work, but do not mark the reference inventory complete or switch to moving main.
- Licensing is not a user preference gate. Preserve actual license/notices for imported source/assets, separate branding from implementation reuse, and do not treat the older AGPL Android fork as relicensed by current upstream Apache.
- No global installation, production service change, secret collection, paid signup, release publication, commit/push or physical-device operation is included in this studio approval. The later app scope allows official runtimes, existing provider accounts and owned emulators under the settled constraints, not arbitrary changes to the user's running environment.

## Source index
All Heeler paths below are relative to https://github.com/ZingerLittleBee/Heeler/tree/17e890481ae5a80e142395ab17974cee8655c6a4 unless already absolute.
- `README.md`, `CHANGELOG.md`, `CONTEXT.md`, `docs/ipad-screenshots.md`, `docs/images/`: feature/reference inventory.
- Under `docs/adr/`: `0011-libssh2-direct-streamlocal-transport.md`; `0024-native-chat-from-agent-transcripts.md`; `0031-herdr-core-hosts-connect-without-starting-a-core.md`; `0032-herdr-core-hosts-fill-the-console-models.md`; `0033-herdr-core-hosts-run-a-heeler-host-helper.md`; `0034-herdr-core-chat-binds-sessions-from-host-evidence.md`.
- `Sources/Heeler/Demo/DemoScreenshotMode.swift`, `Tests/HeelerTests/ChatFixtures/README.md`, `plugin/test-vectors/`: deterministic reference states and fixture provenance, not exercised results.
- https://github.com/kruttan/Heeler/releases/tag/android-0.1.0 — existing native Android fork, commit `8a98b9d123f6abb9e809687a6a9d328c1d74816c`, AGPL, older/incomplete relative to the chosen reference.
- https://github.com/nimrc/bellterm/tree/e13b17a8d1b267f86e944dffa18cee030debd8f4 — native Android client, Apache, useful custom `app/src/main/java/app/bellterm/ui/terminal/TerminalView.kt`; not a complete Heeler-equivalent.
- https://github.com/tuan3w/herdr-mobile — Flutter client; source/docs explicitly limit terminal fidelity and provider verification. Reference only, not the selected UI stack.
- https://github.com/DrKLO/Telegram — public GPL Android application internals, custom Views/Canvas, not a supported standalone Telegram UI SDK. TDLib is not its UI toolkit.
- https://github.com/Lakr233/libghostty-spm/blob/7e45d27160f9b34aca9ca5c9820e9207482f9f04/Ghostty.ref and https://github.com/ghostty-org/ghostty/tree/82938b633ba646db38591d969c3c526332bd7e65 — actual terminal engine pin; inspect `include/ghostty/vt/`, `src/terminal/kitty/graphics_unicode.zig` and `graphics_storage.zig`.
- https://docs.iroh.computer/iroh-services/relays/public and https://docs.iroh.computer/iroh-services/access — accepted public-relay support limits and project-secret restriction.
- https://www.nabto.com/docs/developer/guides/overview/platform-overview.html; https://webrtc.org/getting-started/peer-connections; https://developers.cloudflare.com/realtime/turn/generate-credentials/ — investigated alternatives; not selected, no new services to build.
- https://github.com/mapbox/pixelmatch/releases/tag/v8.0.0 and https://github.com/emikulic/darkhttpd — selected studio comparison and static-serving tools.
- https://developer.android.com/training/testing/other-components/ui-automator; https://developer.android.com/topic/performance/benchmarking/macrobenchmark-metrics; https://perfetto.dev/docs/getting-started/system-tracing — later Android interaction/performance evidence tools, not executed during planning.
