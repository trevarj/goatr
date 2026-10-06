# Goatr

An independent Android facade and Linux companion for stock OMP/Codex sessions
hosted by Herdr. [The implementation plan](plans/initial/GOATR_PLAN.md) is the
source of truth; [AGENTS.md](AGENTS.md) defines collaboration rules.

**Current scope: developer infrastructure only.** The Python namespace is
packageable and its planned non-native dependencies are available. There is no
companion CLI/server, Android application, generated binding, or secure native
artifact yet. In particular, this does not complete native/build package **1A**.

## Start developing

Install Nix with `nix-command` and `flakes` enabled, then run from this checkout:

```sh
nix develop
make check
nix build .#companion --no-link
```

Optional direnv integration: review `.envrc`, then `direnv allow`. It contains
only `use flake`. Before newly created files are tracked by Git, use
`nix develop path:.` and `nix build path:.#companion --no-link`; Git-backed Nix
flakes intentionally ignore untracked files. Do not commit/push without approval.

The default shell uses the exact planned nixpkgs and rust-overlay revisions in
`flake.lock`. It supplies Python (aiohttp, cryptography, segno, packaging tools),
Rust, cargo-ndk, maturin, Git, Make, Ruff, and nixfmt. The lock pins the overlay's
stable Rust selection and the entire Python dependency closure. No global pip,
cargo, rustup, or Android SDK installation is needed. The core shell is defined
for Linux/macOS on x86_64/aarch64; only Linux x86_64 availability was inspected
in this foundation handoff, not cross-platform execution.

Shared commands, also used by [GitHub Actions](.github/workflows/ci.yml):

| Command inside `nix develop` | Purpose |
|---|---|
| `make lint` | Ruff Python lint/import checks |
| `make format` | Format Python and `flake.nix` (modifies files) |
| `make format-check` | Check formatting without modifying files |
| `make smoke` | Direct package/dependency import smoke command |
| `make check` | All non-building core checks |

The Nix `companion` output is the Python package, **not** the planned full
`goatr` executable/native closure. Runtime dependency versions are selected by
the locked Nix graph; project metadata names dependencies without a second
floating pip installation path. New source is read from the working checkout
by `make`/the shell; use Nix package builds to check wheel installation/imports.

## Optional Android tools (Linux x86_64)

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
36.0.0, platform-tools 35.0.2, NDK 28.2.13676358, and command-line tools 21.0.
The emulator is 36.6.11; its shell selects **API 34 only**, while the Android
build shell selects the pinned metadata's **37.0** platform key.
The core checks/package build passed, both optional shells evaluated, and the
Android shell ran JDK21, Gradle9.8.0 and ADB35.0.2. The emulator was not launched.
`ANDROID_HOME`, `ANDROID_SDK_ROOT`, `ANDROID_NDK_ROOT`, `JAVA_HOME`, and the NixOS aapt2 override
are supplied by the shell. The SDK is immutable; never use sdkmanager to install
into its store path. Emulator runtime additionally needs owned AVD/device state
and usable KVM; `/dev/kvm` was not visible on the inspected workstation.

No Gradle modules/wrapper, Maven lock/verification files, or native outputs are
invented here. The exact planned AGP/Kotlin/UI/JNA pins and upstream native
sources resolved during availability preflight; that is not proof of dependency
compatibility, API-37 compilation, secure migration, native loading, or an APK.
See [the preflight record](plans/initial/TECH_STACK.md#foundation-availability-preflight).

## Next handoff

The **1A native/build integration owner** now owns the real secure FFI migration:
fetch/hash the pinned FFI, migrate the whole core/base/relay graph to 1.3.0 with
rustls >=0.23.45, retain patches/Cargo.lock, exclude the named advisories, and
preserve the complete public binding API. Nix must produce matching Python/host
and two-ABI Android outputs plus one source/patch/lock manifest. Only then wire
`:iroh-jvm`, `:iroh-android`, and `:app`, retain Gradle wrapper provenance and
Maven locks/verification, and implement native load/ABI/JNA/alignment gates.
Never use the unexamined published 1.1.0 wheel/AAR as a shortcut.

Freeze package **2A**'s protocol/types/fixtures before any provider or control
consumer. Pairing and authorization follow phases 2–3; real selected-path
framed direct/relay actions cannot be claimed from native loading alone.
[Testing](plans/initial/TESTING.md) distinguishes the current infrastructure
checks from those still-required product gates. Workers hand off without
running overlapping verification; one coordinator integrates and records the
actual checks, artifacts, results, and blockers.

## License and provenance

Independent Goatr code is **GPL-3.0-or-later**; see [LICENSE](LICENSE) and
[NOTICE](NOTICE). No private/reference application source was copied. Retain
applicable upstream notices before importing or distributing third-party code.
