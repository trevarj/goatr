# Tech stack and native transport

Scope: independent repository/build inputs, exact dependency provenance, Rust-native companion, secure Android bindings and the iroh connection lifecycle.

[Documentation](../README.md) · [Pairing/security](PAIRING_SECURITY.md) · [Protocol/state](PROTOCOL_STATE.md) · [Source anchors](../REFERENCES.md#critical-files--anchors)

## Repository and pinned build inputs

Create one repository with `:app`, `:iroh-jvm`, `:iroh-android`, one Rust Cargo package at `companion/Cargo.toml` with `src/lib.rs`/`src/main.rs`, `protocol/`, and reproducible Android binding sources under `third_party/iroh-ffi/`. Companion modules live under `companion/src/`; integration tests and non-installed smoke/native-inventory examples use that same package, not another orchestration crate. The publisher-controlled application ID/namespace is `io.github.trevarj.goatr`, label `Goatr`, debug suffix `.debug`, GPL-3.0-or-later with retained source notices. Source paths and current launch/runner commands use that identity together; historical artifacts retain their original IDs. Debug signing does not establish release signing or distribution readiness. Use constructor wiring, not Hilt or a host transport/plugin layer; do not import MOTD application/IRC/AI modules. These Rust paths are proposed, not files already supplied.

Phase8 additionally owns only `plugins/goatr/herdr-plugin.toml` and executable `plugins/goatr/goatr.sh`: one Linux plugin-v1 action/terminal pane with a Bash dispatch branch, no SDK/framework/helper crate or plugin-specific Rust binary. The dispatcher builds the **same** immutable source-reviewed `packages.goatr`, then invokes the existing absolute Rust setup/pair executable. [Herdr integration](HERDR_INTEGRATION.md#supported-plugin-v1-entrypoint) owns supported manifest/CLI/API fields and public pinned/local-link distribution; [Packaging](COMPANION_PACKAGING.md#retained-build-outputs-not-profiles) owns persistent versioned Nix `--out-link` roots and atomically activated pointers under `$XDG_STATE_HOME/goatr/install`, separate from checkout/config/identity. No profile/global provider/cache installation. Extend the existing companion smoke example for this real lifecycle gate, not another orchestration package.

Use the following source-backed pins:

| Component | Pin |
|---|---|
| Gradle / AGP / Kotlin plugins / JDK | 9.8.0 / 9.4.1 / 2.4.20 / 21 |
| minSdk / compileSdk / targetSdk | 26 / 37 / 36 |
| Android SDK build tools / platform tools / NDK | 36.0.0 / 35.0.2 / 28.2.13676358 (r28c) |
| Compose BOM / activity-compose / lifecycle / core-ktx | 2026.09.00 / 1.13.0 / 2.11.0 / 1.19.1 |
| coroutines / serialization-json | 1.11.0 / 1.11.0 |
| UnifiedPush / CameraX / ZXing | 3.3.5 / 1.6.1 / 3.5.4 |
| Native Markdown | `io.noties.markwon:core:4.6.2` |
| Iroh FFI source | [n0-computer/iroh-ffi](https://github.com/n0-computer/iroh-ffi/tree/5e451092dba0c1a09ee83ff6e5be37b1152a5c58), `5e451092dba0c1a09ee83ff6e5be37b1152a5c58` (1.1.0) |
| Iroh core/base/relay | 1.3.0; core release source [v1.3.0 commit](https://github.com/n0-computer/iroh/tree/0072d7d84b233f9e7185eb676f049beaf557ac03) |
| JNA | `net.java.dev.jna:jna:5.19.1@aar` on Android; matching JAR only for JVM |
| nixpkgs | `767b0d3ec98a143ad9ed7dfc0d5553510ac27133` |
| rust-overlay | `e60029353d0c48d216bc4b065168ccd4079c166f` |

The current Nix flake uses the locked overlay's stable Rust toolchain; the optional Android shells add `aarch64-linux-android` and `x86_64-linux-android` standard libraries, JDK21, cargo-ndk and the pinned Android SDK/NDK. Commit no global tool installation: `.envrc` is exactly `use flake`. Build Android arm64-v8a and x86_64; the latter also supplies emulator proof. The separate emulator shell uses the API-34 default x86_64 image. The companion calls iroh1.3.0 directly from Rust; it needs no host UniFFI binding, Python runtime or maturin build. No OMP/Codex source builds. Obsolete Python tooling/package outputs have been removed. Current source/output/check ownership is documented in AGENTS.md; executed artifact and runtime evidence belongs in [`STATUS.md`](../STATUS.md).

### Companion Rust package and dependency resolution

Use Tokio for iroh and bounded asynchronous Unix sockets/channels/timeouts; `serde`/`serde_json` and `toml` for strict wire/provider values and versioned configuration; `clap` for the finite CLI. Use `reqwest` with default features disabled and a compatible Rustls backend for approved-origin HTTPS publication, and `tokio-tungstenite` with Rustls for OMP WSS and Codex WebSockets over connected Tokio Unix streams. `axum` serves only the bounded companion-local OMP HTTP/WebSocket routes; explicit status/handshake checks, frame limits and queue limits must retain stock behavior, not framework defaults. No general provider server framework.

Use `rusqlite` with system SQLite from locked nixpkgs, plain SQL/transactions and one bounded blocking owner off the transport reader; durable receipt/creation/attention writes finish before dispatch. Use RustCrypto `aes-gcm` for exact stock AES-256-GCM framing, `sha2`/`hmac` for hashes and constant-time keyed verification, and `qrcode` with image features disabled for built-in terminal Unicode rendering. Reuse one compatible OS-backed secure randomness source; endpoint secrets remain owned by iroh. Small `base64`, `uuid` and `url` choices serve only the specified encoding/identity/origin rules. Prefer stdlib filesystem/process primitives and existing compatible stream utilities over extra frameworks; no ORM, connection pool, layered-config system or custom cryptography.

These library choices are **proposals, not resolved version pins**; [primary API references](../REFERENCES.md#rust-library-proposals) are not a Goatr lockfile. Resolve compatible released versions/features against iroh1.3.0 and the locked toolchain, retain `companion/Cargo.lock`, and record exact direct/transitive versions, registry source/checksums or immutable git revisions, TLS/features, system SQLite provenance, licenses and advisory database identity/date in the package manifest. Reuse compatible dependencies from the pinned iroh graph where possible; its manifest requirements alone do not select secure resolved versions. Nix consumes retained locks and fixed-output/vendor inputs for locked/offline package builds, never floating build-time resolution. The Android FFI has its own retained lock as described below.

### Foundation availability preflight

The repository retains these locked source inputs; resolving a lock does not prove a build or runtime gate:

| Input | Locked NAR hash |
|---|---|
| nixpkgs `767b0d3ec98a143ad9ed7dfc0d5553510ac27133` | `sha256-tzMgSkV7kljEkqIjlgV6F+n+xD+/a35Db8bs7a4BFAo=` |
| rust-overlay `e60029353d0c48d216bc4b065168ccd4079c166f` | `sha256-Y8eN7ki0Urx4Ojf4w1xSpWg8uk/3lJqK+E0Oth77rH0=` |

### Native artifact ownership

Nix is the **one native source-fetch/build owner**. Commit `flake.lock`, `companion/Cargo.lock`, `third_party/iroh-ffi/patches/`, the separately migrated `third_party/iroh-ffi/Cargo.lock`, and source revision/hash metadata. Do not vendor a second independently patched FFI tree or let Gradle download/build another native implementation. Gradle dependency locking/verification metadata covers Kotlin/JNA/Android dependencies separately. Companion and Android are two independently locked graphs, not a forced shared Cargo workspace.

| Producer | Input | Output and consumer |
|---|---|---|
| Nix patched-source derivation | Immutable FFI source, patch series, retained FFI lock | One Android source/patch/lock identity for its native outputs and generated Kotlin API |
| Nix `iroh-android` package | That source, UniFFI generator from same FFI lock, NDK/rust targets | Directory containing `jniLibs/arm64-v8a/libiroh_ffi.so`, `jniLibs/x86_64/libiroh_ffi.so`, generated Kotlin source and source/lock manifest; **not** a separately fetched AAR |
| Nix `goatr` package | Single companion Cargo package, retained companion lock, immutable selected core sources and system SQLite | Rust `goatr` executable exposed by `packages.goatr`/`apps.goatr`, direct iroh API calls; no runtime dependency on `iroh_ffi` |
| Optional Nix host FFI output | Same Android FFI source/patch/lock identity, only if actual JVM native tests require it | Explicit JVM test input only, never a companion dependency or Android-packaged host library |
| Gradle `:iroh-jvm` | Nix-generated Kotlin source only | Plain binding classes JAR, no native resources; JNA compileOnly, JVM testRuntimeOnly matching JAR |
| Gradle `:iroh-android` | `:iroh-jvm`, Nix jniLibs and pinned IrohAndroid context seam | Android AAR packaging Rust natives/context helper, JNA5.19.1 AAR supplies `libjnidispatch.so`; exclude plain JNA JAR transitively |
| Gradle `:app` | `:iroh-android` and native UI | APK with exactly intended ABI variants; alignment/loading gate checks Rust and JNA libraries |

Expose immutable Android output as `GOATR_IROH_ANDROID` in the dev shell; Gradle sync/staging only copies/declares these inputs under its build directory, never rebuilds/fetches Rust. Missing/mismatched manifest fails with `nix build .#iroh-android` guidance. Shared generated Kotlin is produced once by Nix, not separately in both modules. Any JVM native smoke uses the optional host FFI output explicitly, never packages it into Android.

Compare the exact shared transport/security provenance in both manifests: selected iroh/core/base/relay1.3.0 version, source/checksum or immutable revision, and intended secure Rustls selection, including duplicate resolved versions. Full lockfiles/platform feature graphs need not match; there is no Rust-host/Android FFI ABI identity claim. Wire/protocol fixture parity and real endpoint interoperability are the cross-consumer gates. Both graphs must meet the Rustls floor and advisory exclusions below; an iroh version label or the upstream compatible Rustls requirement is not evidence.

## Patched native bindings

Fetch the pinned Android FFI source reproducibly during build and maintain only necessary patches/generated-binding wiring in Goatr. Its release lock currently resolves vulnerable core1.0.2: update core/base/relay to1.3.0 and retain its migrated Cargo.lock. Independently resolve the companion's direct iroh1.3.0 graph. Require Rustls >=0.23.45 in both actual graphs; exclude GHSA-7cq4-mhxw-xw78 and GHSA-jx4g-cg2x-jc35, inspecting duplicate versions and recording advisory database identity/date. Do not use the published unexamined1.1.0 AAR as the Android implementation. Adapt FFI source to1.3.0 API differences while retaining its complete Kotlin-facing public API; do not downgrade the core to make it compile.

In fetched `src/endpoint.rs`, add three small Android binding methods: async `Endpoint.network_change()` forwarding core `Endpoint::network_change().await`; `EndpointBuilder.disable_port_mapping()` forwarding `.portmapper_config(PortmapperConfig::Disabled)`; and `EndpointBuilder.clear_ip_transports()` forwarding the same core builder method. The last supports a real relay-only smoke, not a user-visible transport mode. These core1.3.0 APIs were read in `iroh/src/endpoint.rs` at lines516,803 and1671. The Rust companion calls those core methods directly, without an FFI wrapper. Production disables automatic router mapping and uses N0 discovery/public relays. Persist each installation's distinct endpoint secret; shared source provenance never means shared private keys. Build Android natives/generated Kotlin from the same FFI source/patch/lock. Use JNA5.19.1 consistently, excluding its plain JAR on Android. Validate Rust and JNA native libraries and APK16-KiB compatibility; https://github.com/java-native-access/jna/issues/1647 explains the older ARM64 loading failure.

The three extensions are a proposed minimal patch, **not proof that the1.1.0→1.3.0 migration is otherwise source-compatible**. Gate phase1 on compiling the complete generated Kotlin-facing API and Rust companion, rebuilding intended Android native outputs from the FFI manifest, comparing shared transport/security provenance, resolved-advisory checks, Android context JNI linkage and native loading. If core/UniFFI APIs moved, make the smallest source-backed adaptation or stop with the exact incompatibility; no vulnerable published-AAR fallback. Pinned FFI exposes `paths`, `watch_paths` and `watch_path_events`; Android uses them and Rust uses direct core path observation, not another reachability detector.

## Endpoint and stream lifecycle

`GoatrConnection.kt` calls `computer.iroh.IrohAndroid.installAndroidContext(applicationContext)` before endpoints; the pinned `kotlin/android/.../IrohAndroid.kt` source confirms the helper/JNI name, while post-upgrade runtime linkage remains a gate. Register one ConnectivityManager callback per Android endpoint owner, coalesce network-change notifications into the new async native method, unregister and cancel native path watches on shutdown. Keep only applicationContext. The Rust companion runs direct iroh endpoints on Tokio and uses core network-change/path APIs; no host UniFFI event-loop initialization exists.

Use one endpoint per running installation, one reliable bidirectional stream per phone/host connection, ALPN `goatr/1`; no gossip, datagrams, replication or transport plugin layer. Dispose streams/connections/watch handles explicitly; shutdown awaits bounded native closure and recreates endpoint with the retained secret, never a new pairing identity. Companion owns provider reducers independently of phone connections. Duplicate authenticated connections follow [security newest-hello-wins](PAIRING_SECURITY.md#pairing-capability-and-authenticated-handshake); reconnection/sequence recovery follows protocol, not a second transport state machine. Android background handoff closes only its transport after verified push ACK; never desktop providers.

## Acceptance gates

See [native libraries](../ACCEPTANCE.md#native-libraries), [direct and relay transport](../ACCEPTANCE.md#direct-and-relay-transport), [network changes](../ACCEPTANCE.md#background-delivery). These are required implementation checks, not recorded passing results.
