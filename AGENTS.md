# Repository instructions

## Authority and current boundary

Read `docs/README.md`, your assigned topic, its dependencies, and
`docs/REFERENCES.md` before editing. Those documents own product
contracts. Keep README user-facing: product overview, availability, use and
license only. Developer onboarding/tool commands belong in this file;
implementation progress, verification status and next actions belong in
[docs/STATUS.md](docs/STATUS.md). Do not duplicate topic contracts in onboarding or status summaries.
Do not invent product commands, dummy bindings, insecure native fallbacks, or
passing runtime evidence.

## One coordinator, isolated writers

- One named integration owner controls each work package and its merge/check
  handoff. Workers **must not recursively delegate**; only the coordinating
  developer/agent assigns independent slices.
- Give every concurrent writer its own branch and Git worktree, for example
  `git worktree add -b feature/<slice> ../goatr-<slice>`. Inspect existing branches
  and paths first; never overwrite another worktree or another developer's work.
- The coordinator records path ownership and shared-contract owners in the task
  assignment before work begins. A file has one writer. Independent worktrees
  do not grant permission to change another owner's files/contracts.
- Message the owner before touching a shared file. Schema/config/identity/native
  manifest changes land through their owner, with every consumer and matching
  fixture updated in the same handoff. Do not add aliases or parallel definitions
  to avoid coordinating a clean cutover.
- Runtime cleanup requires a candidate manifest reviewed by the integration owner
  before any stop: join retained owned-run/terminal-outcome records to the current
  original process identities. Unit prefixes, UID, executable names or run-ID
  suffixes alone are insufficient. Keep unproved resources untouched.
- Process-only cleanup may prove descendants through complete, start-ordered
  in-cgroup ancestry to the original owned root. Pin each process's own namespace
  tuple; sandbox descendants need not share the root's namespaces. This ownership
  proof grants no native binding, session adoption or interaction authority.
- Never unescape an entire source file to repair generated patches. Restore the
  exact pre-edit bytes and reapply only the bounded change; preserve existing
  string, byte, character and shell-command escapes.
- Before retiring a live controller, verify its replacement config is an owned
  0600 regular file in a 0700 directory, and its controller and Codex-owner unit
  names are distinct. Include file modes in the prepared-resource proof.
- Every mutating command must set its intended workdir explicitly and verify
  the expected absolute cwd before editing; never rely on an inherited cwd.
- Before editing an existing untracked file, retain an exclusive private
  pre-edit copy and its hash so scoped changes can be removed without losing
  earlier native work. Never overwrite a previous backup.
- Commits, pushes, paid signup, service/global-tool installation, release keys,
  private-source copying, production resources, and physical-device use require
  explicit authorization. The project-scoped SDK consent in the optional Nix
  shells is already authorized; it is not permission to change global settings.

## Shared ownership map

These are phase ownership boundaries, not claims that future files exist.

| Owner | Paths/contracts |
|---|---|
| Foundation / 1A native-build | `flake.nix`, `flake.lock`, `.envrc`, `Makefile`, `.github/`, root notices; `companion/Cargo.toml`/`Cargo.lock`, Rust product Nix/Make/CI gates, Gradle root/wrapper, `third_party/iroh-ffi/` with its own lock, `iroh-jvm/`, `iroh-android/`, native artifacts/provenance comparison |
| 2A protocol | `protocol/`, canonical wire schema/types/hash fixtures and Rust/Kotlin protocol consumers; owns every shared wire name and strict raw-token/duplicate-key/canonicalization rules |
| 2B state/storage | Serialized reducers, generation/receipt/content/cache/read state and finite retention; coordinate shared Android/companion models with 2A |
| 3 security | Pairing/admin/authorization/revocation contracts and key-store lifecycle; CLI arguments must match packaging contracts |
| 4 topology/config | Configuration/trust roots, runtime-path/wrapper resolution and Herdr session identity/creation contracts |
| 5A OMP / 5B Codex | Separate provider-specific adapter paths only after 2–4 inputs are frozen |
| 6 Android facade | `app/` UI/lifecycle, excluding shared wire/state/native ownership above |
| 7 delivery | Foreground/UnifiedPush delivery seams, attention ownership shared with 2B and revocation with 3 |
| 8 integration/packaging | Final Rust executable/setup/service packaging, `plugins/goatr/herdr-plugin.toml` and one Bash dispatcher; retained owned Nix output/activation/upgrade/removal lifecycle outside Herdr checkouts; same-binary internal Codex relay, existing companion Rust smoke/native-inventory examples and integrated evidence |

A topic edit belongs to its contract owner. The coordinator owns cross-topic
plan status, README/AGENTS updates, and resolving overlapping assignments.

## Development and handoff

### Current foundation onboarding and commands

Install Nix with `nix-command` and `flakes` enabled, then run from this checkout:

```sh
nix develop
```

Optional direnv integration: review `.envrc`, then `direnv allow`. It contains
only `use flake`. Before newly created files are tracked by Git, use
`nix develop path:.`; Git-backed Nix flakes intentionally ignore untracked files.
For large checkouts, `path:.` also copies ignored build outputs before package
filtering. Use a recorded, hash-verified snapshot of all extant tracked and
nonignored untracked source files for clean builds; preserve file modes, locks
and the Git index. Never reset or discard native work to prepare build inputs.

Use the locked shell, not global cargo/rustup/SDK installs. The default shell
uses the exact nixpkgs and rust-overlay revisions retained in `flake.lock` and
supplies stable Rust (Cargo, rustc, rustfmt and Clippy), cargo-ndk, cargo-audit,
pkg-config, SQLite, jq, unzip, Git, Make and nixfmt. The core shell is defined for
Linux/macOS on x86_64/aarch64; execution evidence belongs in [docs/STATUS.md](docs/STATUS.md).
Python packaging/runtime/tooling is removed; use the single companion Cargo
package and independently locked Nix Android producer.

Current shared development-tool commands, also used by
[GitHub Actions](.github/workflows/ci.yml):

| Command inside `nix develop` | Purpose |
|---|---|
| `make format` | Format `flake.nix` and companion Rust (modifies files) |
| `make format-check` | Check Nix/Rust formatting without modifying files |
| `make smoke` | Report Cargo, rustc, rustfmt and Clippy versions |
| `make rust-check` | Locked Cargo tests, including example security/native regressions, and all-target Clippy |
| `make audit` | Audit both retained Cargo locks against the pinned database |
| `make check` | Formatting, tool smoke, Rust checks and both audits |
| `make build` / `make native` | Build the companion / Android native Nix output |

These checks exercise the working checkout, not every product acceptance gate.
The frozen advisory database's coverage limits and supplementary exclusions are
recorded in `third_party/iroh-ffi/source.json`; an audit pass alone is not proof
of every required exclusion. Follow [Testing](docs/ACCEPTANCE.md) for
native loading, framed transports, live providers and actual APK/UI checks.
Current observed verification and remaining work live in [docs/STATUS.md](docs/STATUS.md).

### Rust and native product development

Follow [Tech stack](docs/contracts/TECH_STACK.md#companion-rust-package-and-dependency-resolution)
for one companion Cargo package, direct iroh1.3.0 and a retained
`companion/Cargo.lock`. Android retains its independently migrated FFI lock,
generated Kotlin/context/JNA/ABI requirements; compare exact shared
transport/security provenance, not entire locks or host/Android ABI. Host FFI
is optional for actual JVM native tests only, never a companion dependency.
Resolve compatible exact new crate versions/features during implementation and
retain sources/checksums/licenses/advisory evidence; do not turn latest API
documentation or upstream manifest ranges into fabricated application pins.

Use `path:.` while new files remain untracked. Current package commands:

```sh
nix build path:.#goatr path:.#iroh-android
nix develop path:. -c cargo test --locked --manifest-path companion/Cargo.toml --all-targets
nix develop path:. -c cargo fmt --manifest-path companion/Cargo.toml -- --check
nix develop path:. -c cargo clippy --locked --manifest-path companion/Cargo.toml --all-targets -- -D warnings
```

The non-installed transport smoke exercises the actual packaged companion.
Set `GOATR_BIN` to the absolute `bin/goatr` from the selected Nix build output
and `EVIDENCE_DIR` to an owned private evidence directory, then run:

```sh
nix develop path:. -c cargo run --locked --manifest-path companion/Cargo.toml --example smoke -- \
  --transport direct --goatr "$GOATR_BIN" --evidence-dir "$EVIDENCE_DIR"
```

Repeat with `--transport relay` for the public N0 relay gate. Follow Testing's
resource ownership rules; transport-only success is not live-provider/UI proof.

Make/CI share the implemented Rust product gates. Native-inventory and owned
smoke/UI/background runners belong as non-installed examples in the same package;
[Testing](docs/ACCEPTANCE.md#implemented-checkpoint-commands-and-final-proposed-owners) owns their exact
commands and unchanged acceptance gates. Do not create another helper package
or orchestration framework. Internal `goatr codex-relay --binding PATH` validates
a private native binding and uses the same packaged binary in an independent
service, never a `serve` lifetime tie; it rejects the companion admin override.

### Optional Android tools (Linux x86_64)

Compose helpers under `app/src/main/kotlin/io/github/trevarj/goatr/ui/` declare
`package io.github.trevarj.goatr`. Resolve imports from package declarations;
the directory does not define a `.ui` package.

Kotlin `when` branch bodies end without commas. Check newly written branches
for `-> { ... },` before handoff; that is invalid Kotlin syntax.

The project owner explicitly accepted the **Android SDK license** for this
project. Selecting either shell below opts into that project-scoped acceptance;
no global Nix configuration or host license settings are changed. The core
shell and CI do not pull in the SDK/NDK. Android tools are nonfree and keep their
upstream license; do not publish an SDK cache without reviewing those terms.

```sh
nix develop .#android    # JDK 21, Gradle 9.8.0, SDK 37, NDK r28c
nix develop .#emulator   # API-34 default x86_64 image + emulator
```

Use `path:.#android` / `path:.#emulator` before Git tracks the new files.
Both shells provide the Android Rust target standard libraries, build-tools
36.0.0, platform-tools 35.0.2, NDK 28.2.13676358 and command-line tools 21.0.
The emulator is 36.6.11; its shell selects **API 34 only**, while the Android
build shell selects the pinned metadata's **37.0** platform key.
`ANDROID_HOME`, `ANDROID_SDK_ROOT`, `ANDROID_NDK_ROOT`, `JAVA_HOME` and the
NixOS aapt2 override are supplied by the shell. The SDK is immutable; never use
sdkmanager to install into its store path. Emulator runtime additionally needs
owned AVD/device state and usable KVM.

Before downloading/building their closures, evaluate the `android` and
`emulator` shell derivation paths using the commands in
[Testing](docs/ACCEPTANCE.md#current-infrastructure-checks).
For build pins and source availability, read
[Tech stack](docs/contracts/TECH_STACK.md#foundation-availability-preflight);
availability is not dependency compatibility or native/product evidence.

### Owned OMP diagnostic authorization

The owner authorizes fresh owned OMP creation/source-loss diagnostics without repeated approval.
Capture requires explicit opt-in and a validated owned relay, one shared counter
with at most eight sanitized closed phase/guard lines, and no extra native reads,
RPCs, probes, action replay, deadline changes or identity/permission relaxation.
For the current blocker work, the owner also authorizes a private Herdr build
and a private receiver diagnostic channel; installed Herdr, wrappers and global
settings remain unchanged. Do not repeat the resolved scope-approval question.
Private Herdr diagnostic builds must retain the production `herdr` config/socket
namespace: use the package-only `profile.dev.package.herdr.debug-assertions=false`
build override, with version/manifest update checks disabled only in the run’s
private Herdr config. Default dev builds use `herdr-dev` and fail the owned socket gate.
Owned private Creation diagnostics may use the approved 60-second startup budget
only when capture is requested and its owned receiver is validated; ordinary
startup remains 20 seconds and provider deadlines stay unchanged.
This does not authorize pairing.rs capture edits. Remove temporary capture after
exercising the causal fix.
Capture-only edits must preserve each authorization guard's exact lexical scope.
Absolute creation deadlines are rechecked inside native admission validators
after intervening waits; a check before an await does not cover admission.
The owner additionally approved one fresh owned launch with a process-local
diagnostic copy of the stock OMP reporting hook and isolated diagnostic output/read.
Preserve effective extension lists, verify exactly one reporter, retain the shared
eight-line limit and existing native calls/deadlines; leave installed files unchanged.

The owner subsequently authorized all necessary scoped diagnostic iterations and
checks through a working pass, without further approval questions. This includes
bounded native startup-log reads and fresh process-local hook captures. Preserve
identity, native permission, privacy, resource ownership and no-replay protections.

The explicit owned Codex startup Terminal feature check may perform one
post-settlement inventory and one `pane.read_bound`, using an immutable runtime
pin from the existing prelaunch snapshot. It requires the separate helper flag,
owned Codex consent/relay and hash-verified private Herdr artifact. Ordinary
diagnostics retain their no-extra-RPC boundary. No unguarded read, replacement
adoption, retry, raw terminal text/hash/capture or additional report budget.

## Verification and handoff discipline

Use incremental builds and checks scoped to changed behavior. Development APKs
may be handed over for feedback with known failures disclosed; full clean builds
and exhaustive acceptance belong to a later checkpoint. Keep identity,
permission, privacy and no-replay protections intact. See `docs/STATUS.md`.

For the approved mobile MVP, use one GPT-6.1 Sol medium implementation writer
and an independent GPT-6.1 Sol high reviewer before signed feature commits.
Run targeted regressions and actual functional checks on the authorized device
after each reviewed handoff; exhaustive clean/release checks remain later.
Preserve app data and pairing on APK updates; exclude pairing secrets and
sensitive screens from screenshots or video. See docs/UI_POLISH_PLAN.md.

ADB shell invokes a remote shell: quote each remote token explicitly, including
labels and JSON. Owned UI controllers must budget pre-input release admission
separately from their UI-action deadline. Resolve an APK's registered launcher
component before starting it; a debug application ID is not its Activity namespace.
Window identity parsers must distinguish unavailable formats from a proven
foreground mismatch and preserve secure-window checks across vendor metadata.

Workers do not run builds, tests, linters, formatters, or smoke checks mid-flight
unless verification is their explicit assignment. After all owned changes land,
one integration owner runs relevant checks once. Current shared checks exercise
Rust tests/Clippy, formatting and retained-lock audits, not all runtime gates:

```sh
nix develop path:. -c make check
```

Agent-led builds are limited to two cores/jobs and one heavy build at a time.
Use `nix build --cores 2 --max-jobs 1` and Cargo `-j 2` or lower. Android
Gradle builds use `--max-workers=2 --no-parallel` and process-local
`JAVA_TOOL_OPTIONS=-XX:ActiveProcessorCount=2`; Android native Nix/Cargo
builds obey the same two-core limit. Do not change global build settings.

Follow `docs/ACCEPTANCE.md` for implemented native/API/codec checks,
still-proposed runtime/UI runners and exact owned-resource commands. A docs-only
plan change needs documentation consistency review, not unrelated executable
checks beyond the documentation scope.
Never manufacture a pass, silently skip a selected
backend, log secrets or touch another user's sessions/state.
For automation failures, resolve the exact tested artifact's failure line before
interpreting retained snapshots. A first-poll snapshot is not the final receipt
state or proof of which later assertion failed.

Every handoff names changed paths, exact executed commands/results (or clearly
unexecuted coordinator checks), source/lock identity, artifacts, actual behavior
and precise blockers. No build/runtime/advisory/16-KiB claim without its own
observed evidence. Source/URL availability alone is not compatibility evidence.

