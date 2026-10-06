# Repository instructions

## Authority and current boundary

Read `plans/initial/GOATR_PLAN.md`, your assigned topic, its dependencies, and
`plans/initial/REFERENCES.md` before editing. Those documents own product
contracts. Keep README user-facing: product overview, availability, use and
license only. Developer onboarding/tool commands belong in this file;
implementation progress, verification status and next actions belong in the
main plan. Do not duplicate topic contracts in onboarding or status summaries.
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
- Commits, pushes, paid signup, service/global-tool installation, release keys,
  private-source copying, production resources, and physical-device use require
  explicit authorization. The project-scoped SDK consent in the optional Nix
  shells is already authorized; it is not permission to change global settings.

## Shared ownership map

These are phase ownership boundaries, not claims that future files exist.

| Owner | Paths/contracts |
|---|---|
| Foundation / 1A native-build | `flake.nix`, `flake.lock`, `.envrc`, `Makefile`, `.github/`, root notices; current-only superseded `pyproject.toml`; future `companion/Cargo.toml`/`Cargo.lock`, Rust package/Nix/Make/CI cutover, Gradle root/wrapper, `third_party/iroh-ffi/` with its own lock, `iroh-jvm/`, `iroh-android/`, native artifacts/provenance comparison |
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
`nix develop path:.` and `nix build path:.#companion --no-link`; Git-backed Nix
flakes intentionally ignore untracked files.

Use the locked shell, not global pip/cargo/rustup/SDK installs. **The following
describes the executable, superseded Python foundation only**, not the intended
Rust companion. The default shell uses the exact planned nixpkgs and
rust-overlay revisions in `flake.lock` and still supplies Python (aiohttp,
cryptography, segno, packaging tools), Rust, cargo-ndk, maturin, Git, Make, Ruff
and nixfmt. The lock pins the stable Rust selection and current Python closure.
The core shell is defined for Linux/macOS on x86_64/aarch64; execution evidence
belongs in the main plan. Python/maturin are not future companion requirements.

Current-only shared commands, also used by
[GitHub Actions](.github/workflows/ci.yml) until the actual Rust cutover:

| Command inside `nix develop` | Purpose |
|---|---|
| `make lint` | Ruff Python lint/import checks |
| `make format` | Format Python and `flake.nix` (modifies files) |
| `make format-check` | Check formatting without modifying files |
| `make smoke` | Direct package/dependency import smoke command |
| `make check` | All non-building core checks |

The existing Nix `companion` output is the superseded Python package, **not**
the planned Rust `goatr` executable/native closure. Its dependency versions are
selected by the locked Nix graph, without a second floating pip install path.
Current `make`/shell checks read the working checkout and Nix package builds
check wheel installation/imports. Leave these descriptions truthful until the
actual code/tooling migration; dependency imports prove neither Rust behavior
nor transport/product acceptance.

### Intended Rust development after migration

Follow [Tech stack](plans/initial/TECH_STACK.md#companion-rust-package-and-dependency-resolution)
for one companion Cargo package, direct iroh1.3.0 and a retained
`companion/Cargo.lock`. Android retains its independently migrated FFI lock,
generated Kotlin/context/JNA/ABI requirements; compare exact shared
transport/security provenance, not entire locks or host/Android ABI. Host FFI
is optional for actual JVM native tests only, never a companion dependency.
Resolve compatible exact new crate versions/features during implementation and
retain sources/checksums/licenses/advisory evidence; do not turn latest API
documentation or upstream manifest ranges into fabricated application pins.

The following are **future implementation commands**, not runnable or verified
until the Rust package/lock/Nix outputs exist:

```sh
nix build .#goatr .#iroh-android
nix develop -c cargo test --locked --manifest-path companion/Cargo.toml
nix develop -c cargo fmt --manifest-path companion/Cargo.toml -- --check
nix develop -c cargo clippy --locked --manifest-path companion/Cargo.toml --all-targets -- -D warnings
```

At that actual cutover, migrate Make/CI to these Rust package gates and retire
Python packaging/runtime/maturin checks. Native-inventory and owned smoke/UI/
background runners are non-installed examples in the same companion package;
[Testing](plans/initial/TESTING.md#proposed-command-suite) owns their exact
commands and unchanged acceptance gates. Do not create another helper package
or orchestration framework. Internal `goatr codex-relay --binding PATH` uses the
same packaged binary in an independent service, never a `serve` lifetime tie.

### Optional Android tools (Linux x86_64)

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
[Testing](plans/initial/TESTING.md#current-infrastructure-checks).
For build pins and source availability, read
[Tech stack](plans/initial/TECH_STACK.md#foundation-availability-preflight);
availability is not dependency compatibility or native/product evidence.

### Verification and handoff discipline

Workers do not run builds, tests, linters, formatters, or smoke checks mid-flight
unless verification is their explicit assignment. After all owned changes land,
one integration owner runs relevant checks once. Current-only superseded
foundation checks (not Rust or documentation verification) are:

```sh
nix develop -c make check
nix build .#companion --no-link
```

Follow `plans/initial/TESTING.md` for future Rust/native/product gates and exact
owned-resource commands. A docs-only plan change needs documentation
consistency review, not execution of missing Cargo/native commands or rerunning
historical Python evidence. Never manufacture a pass, silently skip a selected
backend, log secrets or touch another user's sessions/state.

Every handoff names changed paths, exact executed commands/results (or clearly
unexecuted coordinator checks), source/lock identity, artifacts, actual behavior
and precise blockers. No build/runtime/advisory/16-KiB claim without its own
observed evidence. Source/URL availability alone is not compatibility evidence.

