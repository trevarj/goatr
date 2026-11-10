# Repository instructions

## Current product and authority

Goatr is the Android Herdr client imported from Zagal, with SSH and authenticated
iroh transports. README.md owns user setup; this file owns architecture/developer
commands. MOBILE_HERDR_MVP_PLAN.md is historical evidence, not the current product
contract. Its installed-only native restriction and Close omission are superseded
by the user-approved Zagal cutover.

The project may package and execute unchanged upstream Herdr Core `hc` 0.1.19.
Never patch Herdr, replace a globally installed binary, adopt existing sessions for
checks, or infer that the old installed Herdr 0.9.1 implements `hc connect`.
Full terminal/command/upload/tab-close/space-close authority is intentional ONLY
following fresh full-control HC pairing. Old peer grants do not imply that consent.

## Architecture and security

- `app/`: Goatr-branded native Android UI, settings and lifecycle; application id
  `io.github.trevarj.goatr`, debug `.dev` (does not replace old `.debug` installs).
  Source namespace is `io.github.trevarj.goatr.app`; retained security/wire types
  remain in `io.github.trevarj.goatr`.
- `core/`: upstream Herdr client/framing, JVM target and common tests.
  `herdr.core.Duplex` is the sole transport boundary. Terminal/session/action ids
  retain their original host prefix; never retarget an old action after selection.
- `ssh/`: pinned SSH host trust and protected phone SSH key; no trust-on-error.
- `terminal/`: Ghostty native Android View/JNI. Its internal `dev.herdr.terminal`
  package/JNI registration stays consistent; it is a library namespace, not an app id.
- `companion/`: retained private pairing/admin/SQLite/iroh security. `serve --hc`
  bypasses the legacy topology completely and advertises only `goatr/hc/1`.
  Legacy `serve`/`goatr/2` remains a separate intentional companion interface, not
  an adapter or fallback for the new Android product.
- `goatr/hc/1`: exactly one bidirectional stream, existing Goatr envelope2 framed
  `pair.confirm` (optional for already HC-authorized peers), then `hello`. Verify
  authenticated iroh identity and ALPN. Only after successful Hello response do the
  same stream halves become raw `hc connect` stdin/stdout. No extra stream or
  remotely selected argv/executable. Native `hc` negotiates generation1/framed
  hello and its own nine-byte multiplexed frames inside that authorized stream.
- Fresh HC invitation redemption atomically stores `hc_grants(peer,epoch)` with
  pairing. Old peers must preview/Confirm a fresh invitation; endpoint/device
  identity remains retained. Revocation invalidates that epoch and closes the
  active connection/connector, not desktop jobs. Recheck the active security guard
  at spawn and each bounded I/O boundary; never retain a guard for an entire session.
- Preserve 300-second single-use invitations, preview separate from Confirm,
  protected no-backup Keystore identity, malformed-storage fail-closed behavior,
  bounded queues/backpressure, cancellation and no automatic input replay.
  Never log invitation/token/key/terminal bytes. A fixed relay is a built-in home
  candidate restriction, not a geographic confinement of every connection.
- Pair-confirm response loss may recover only through authenticated known-peer
  hello, never token replay. If recovery fails, discard the preview and require
  a fresh invitation. Navigation/disposal cancellation must still propagate.
  Adapter-local deadlines are ordinary I/O failures so the session owner's
  existing failure/retry policy runs; they are not caller cancellation.

## Pinned provenance

- Zagal source: `1e73cd24d2abc049c5859a2865c5fa4659a0f5b6` from
  https://github.com/Osc100/zagal (source checkout remains untouched).
- Ghostty: `0fa3d577a5614150ff8e2c152de5fb9d5c6d2dec`, archive unpacked NAR
  `sha256-XHatB+PU7Os1/inapleBdUThGCAGhRnqT0o0EWpNa3s=`.
  Reuse its generated `deps/packages.nix`, including transitive fixed hashes;
  Gradle stages sources in `terminal/build/native-source` and uses Zig `--system`.
- Zig 0.17.0 official x86_64-linux archive SHA-256:
  `1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026`.
  Terminal NDK 27.1.12297006 and retained iroh NDK 28.2.13676358 are both supplied
  by the project Nix SDK; no global SDK installation or guessed compatibility bump.
- `hc` is an OFFICIAL PREBUILT, unchanged upstream 0.1.19 archive, not a claimed
  source build. Source tag revision: `06793ede1bedb1fed8d22d3cae476d3eb239edf0`.
  x86_64-linux archive SHA-256:
  `4adcfb423d52f24c5cc4a1e2cd2fcad1f62d28bb77876ac472c29082a3eaa424`.
  Pins come from https://releases.herdr.dev/core/0.1.19/manifest.json ; that HTTPS
  checksum manifest is not independently signed. Nix retains its license/notices.
- `flake.lock` retains nixpkgs `767b0d3ec98a143ad9ed7dfc0d5553510ac27133`
  and rust-overlay `e60029353d0c48d216bc4b065168ccd4079c166f` unchanged.
  Retain both Cargo locks and existing shared iroh/rustls provenance checks.

## Upstream Zagal maintenance

Before substantial Android/terminal/client fixes or features, and before releases,
check https://github.com/Osc100/zagal against the imported source baseline above.
Use `gh` to verify the current default branch/head, compare commits/diffs, and
inspect relevant releases, merged PRs and issues for existing fixes/features.
The check informs the assigned work; it does not authorize unrelated integration.
Read-only starting point (run from this repository root):

```sh
upstream=Osc100/zagal
base=1e73cd24d2abc049c5859a2865c5fa4659a0f5b6
branch=$(gh api "repos/$upstream" --jq '.default_branch | @uri') &&
head=$(gh api "repos/$upstream/commits/$branch" --jq '.sha') &&
gh api "repos/$upstream/compare/$base...$head" --jq '{status, ahead_by, behind_by, total_commits, html_url}'
gh release list --repo "$upstream" --limit 5
gh pr list --repo "$upstream" --state merged --limit 10
gh issue list --repo "$upstream" --state all --limit 10
```

These bounded lists are discovery, not exhaustive review: inspect relevant
commit/PR diffs and release notes, search task-specific issues, and paginate when
needed. `/home/trev/Workspace/zagal` is an untouched read-only reference checkout,
not automatically current upstream. If `gh`/network access fails, report that
limitation and label local evidence stale; never claim a fresh upstream review.
Do not print auth tokens or private remote URLs.

Goatr imported source into its existing history; shared fork ancestry is not
guaranteed. Default to selective ports/adapted cherry-picks, accounting for copied
or renamed paths. Never blindly `git pull`, rebase onto Zagal, or reset Goatr to it.
A full rebase requires proven shared ancestry, an integration plan and explicit
history-rewrite authorization; preserve dirty work and existing branches.

Preserve Goatr branding/assets/application ids and existing installs; its Rust
companion, iroh pairing/pins, fresh HC grants, no replay and revocation; SSH trust
and secret storage; Nix runners/pins and strict Gradle locks/verification; Kotlin/JNI
package compatibility; and compatibility with unchanged pinned `hc`. Do not blindly
import SDK/crate/native bumps or signing/distribution config, or weaken protections.
Assess schema, HC protocol and native changes before moving pins: availability is
not compatibility evidence. Preserve Apache Zagal license/notices.

After actual ports, record exact upstream SHAs and exclusions in the authorized
commit/handoff. Update provenance only for integrated changes; never advance the
import baseline over unintegrated commits. Run the relevant existing checks,
real changed-path smoke checks and ordinary advertised Git-flake commands under
the ownership rules below; temporary snapshots alone do not prove user commands.

## Commands (repository root)

Use at most two build jobs. APK apps use the caller's writable checkout, not the
immutable Nix source. Outputs are printed and remain under `app/build/outputs/apk`.
`apk-debug` and `apk-release-unsigned` only build; they never install or sign a
release. The explicitly selected `install-*` apps below modify a physical Android
device. No command automatically adopts an existing desktop session.

```sh
nix run --cores 2 --max-jobs 1 .#apk-debug
nix run --cores 2 --max-jobs 1 .#apk-release-unsigned  # not installable until distributor signing
nix run --cores 2 --max-jobs 1 .#install-debug
nix run --cores 2 --max-jobs 1 .#install-release -- --keystore /path/to/release.keystore
nix build --cores 2 --max-jobs 1 .#iroh-android .#goatr
nix develop --cores 2 --max-jobs 1 .#android -c gradle --no-daemon --max-workers=2 --no-parallel :core:jvmTest :app:testDebugUnitTest :terminal:testDebugUnitTest --dependency-verification strict
nix develop --cores 2 --max-jobs 1 -c cargo test --locked --manifest-path companion/Cargo.toml -j 2 --all-targets
nix develop --cores 2 --max-jobs 1 -c cargo test --locked --manifest-path companion/Cargo.toml -j 2 --test hc_bridge real_hc_pair_stream_legacy_refusal_and_active_revocation -- --ignored --test-threads=1
```

Installers accept `--serial SERIAL`; release additionally requires `--keystore PATH`
and accepts `--key-alias ALIAS`. Unknown, missing, empty and duplicate arguments
fail. Check usage without a build or device access:

```sh
nix run .#install-debug -- --help
nix run .#install-release -- --help
```

Without `--serial`, native ADB `-d get-serialno` must select one authorized USB
device; the result is pinned for every subsequent query and install, regardless
of `ANDROID_SERIAL`. Explicit serials may select a physical wireless ADB device.
Installers reject emulator serials, require online authorized state and check
`ro.kernel.qemu` before building and again before installation. They never launch
an emulator, switch targets, uninstall, clear data, or force a downgrade:
installation is only `adb -s SERIAL install -r APK`. Device checks are read-only
until that explicit install; physical-device use still requires permission.

Release installation requires a readable, existing user-supplied keystore. Never
generate release keys, put them in the repository/Nix store, or read/log secrets.
The installer uses pinned SDK `zipalign` to check the unsigned APK before signing,
then `apksigner sign --ks PATH` (optionally `--ks-key-alias ALIAS`) with default
interactive password prompts. After argument/keystore validation, release requires
TTY stdin and stdout before any device access or build, preventing non-console
password input. `--help` remains usable without a TTY; debug has no TTY requirement.
No password arguments/environment are used. Keep
the unsigned artifact at `release/app-release-unsigned.apk`; signed output is
`release/app-release.apk`. Both installers run `apksigner verify` before installing;
debug output is `debug/app-debug.apk`. Signing/verification failure never installs.
An existing release requires its matching signing key; signature mismatch must
fail without an uninstall fallback. The unsigned builder still unsets legacy
`GOATR_KEYSTORE`/`GOATR_PASSWORD` and never signs. Installer JDK, ADB, apksigner and
zipalign come from the existing pinned Nix Android environment.

The last test launches only its own fresh private upstream core and real iroh
endpoints; it checks legacy-grant refusal, fresh consent, raw native hello and
terminal.list, then active revocation. `GOATR_HC` is supplied by the core shell;
never point the test at an existing socket/session. It creates no desktop terminal.

Core's optional `IntegrationTest` is skipped unless `HERDR_IT=1`; enable it only
with an explicitly owned core and terminal. A default JVM pass is not that native gate.

Use `nix run .#desktop` for actual user-selected full-control bridge operation;
`nix run .#hc -- ...` is the independent pinned upstream command. The companion
wrapper defaults `GOATR_HC` to that exact output; an override must be an absolute,
locally chosen compatible executable. HC_SOCKET/HC_SESSION select the native target
locally and must match the intended upstream Herdr session.

All Gradle modules enforce strict dependency locks and retained SHA-256 verification.
For intentional dependency changes, the root build registers `resolveDependencies`
in every project; unqualified task selection resolves them all. Use pinned Nix
Gradle 9.8.0:

```sh
nix develop --cores 2 --max-jobs 1 .#android -c gradle --no-daemon --max-workers=2 --no-parallel --write-locks --write-verification-metadata sha256 resolveDependencies
```

Review new coordinates/hashes rather than treating first-use hashes as independent
trust. Never disable verification to make a build pass. Retain the reviewed module,
buildscript and settings locks with the verification metadata. Existing approved
artifact hashes were preserved when resolving the imported dependency graph.

Nix Git flakes do not include untracked files. Register new package inputs with
`git add -N -- PATH...` before building them; this records intent without staging
their contents. Preserve existing staged content and user changes.
Source-only temporary snapshots are useful for isolated checks, but do not prove
that the advertised Git-flake commands work. Verify those commands directly.
Avoid `path:.` on a checkout containing huge ignored caches: copying occurs before
a package cleanSource filter. Never clean/move targets, Gradle caches, private
evidence or unrelated ignored runtime files.

## Work and check ownership

One named writer owns each shared path/contract; coordinator owns integration and
checks. Workers do not recursively delegate or run mid-flight builds/tests/formatters.
Verify explicit cwd before mutations. Existing untracked files require an exclusive
private preimage and hash before editing. Never commit/push, install global tools,
create signing secrets, change services or use physical devices without permission.
Runtime cleanup requires original owned process identity, not a matching name/UID.
Use single-file rustfmt with `--config skip_children=true` for scoped Rust edits,
not a package-wide rewrite. Report only checks actually exercised. Missing native,
Gradle or physical validation is not a passing acceptance result.
