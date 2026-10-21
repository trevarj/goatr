# Linux companion packaging and setup

Scope: explicit human-invoked setup, CLI/Nix packaging, canonical local configuration and provider-preserving process/service ownership. Read configuration/trust roots before implementing discovery or launch.

[Documentation](../README.md) · [Tech stack](TECH_STACK.md) · [Pairing and administration](PAIRING_SECURITY.md) · [Herdr integration](HERDR_INTEGRATION.md) · [OMP launch](OMP_BACKEND.md#local-relay-and-new-session-launch) · [Codex services](CODEX_BACKEND.md#independent-owner-and-relay-services)

## Packaging and explicit setup

Provide `goatr serve`, `goatr setup`, pairing/revocation commands and Nix `packages.goatr`/`apps.goatr` from the single Rust package/retained lock in [Tech stack](TECH_STACK.md#companion-rust-package-and-dependency-resolution). The same packaged binary has internal `codex-owner --record PATH` and `codex-relay --binding PATH` subcommands. The owner entry clears all four inherited capability sets synchronously before verification or threads and execs only the admitted private launch record’s original wrapper/arguments; its exact service proof is owned by [Codex](CODEX_BACKEND.md). The relay is invoked as its own independently supervised service, never tied to `serve` lifetime. Human-invoked setup validates installed Herdr/OMP/Codex compatibility, configured paths and wrapper launch resolution before offering optional user-service installation/start. Declining service startup prints `goatr serve` then `goatr pair` instructions; pairing does not silently start it. Setup never installs/replaces providers, signs up for infrastructure, kills sessions or relaunches embedded Codex. Re-run validates and preserves identity/project IDs and identical owned files; changed configuration is shown for explicit acceptance. Managed/symlinked config/unit conflicts fail rather than overwrite. Production setup is not authorized by this plan.

## Herdr plugin install and companion lifecycle

Phase8 provides the optional Linux [plugin-v1 entrypoint](HERDR_INTEGRATION.md#supported-plugin-v1-entrypoint): `plugins/goatr/herdr-plugin.toml` and one executable Bash dispatcher `goatr.sh`. This is a convenience front door to the same Rust package/setup/pair flows, not a second installer daemon, SDK or Cargo package. Plugin link/install/enable/Herdr startup never builds, installs, starts or pairs the companion. Only the explicitly opened interactive installer pane offers those operations.

### Source selection and preflight

- Inspect an existing `goatr` and its service/config ownership first. An externally installed or declaratively managed binary/unit/config remains externally owned: offer its compatible absolute executable for existing setup/pair, or report the exact manager-owned change needed. Missing Nix does not prevent using that compatible installation. Never shadow, adopt, replace or remove it merely because its name matches. Conflicting/unproven ownership blocks owned installation; no automatic second daemon/admin socket.
- For an owned install, check Linux/architecture, actual deployed Herdr plugin API, Bash/Git/`flock`, and a supported Nix command/version contract with `nix-command`/`flakes`, local filesystem store/daemon permissions, network and private writable installation roots. Nix2.28 documentation is inspected evidence, not a selected/tested minimum. Remote-only stores cannot satisfy local executable/output-root retention. Missing prerequisites produce visible diagnosis and manual/declarative guidance, never `sudo`, `curl|sh`, global tool installation, Nix configuration/trusted-key/cache changes or provider mutations.
- Production source is the public `trevarj/goatr` repository at a **reviewed immutable release commit**, with the retained flake/Cargo locks and `packages.goatr` build output. Obtain and validate the managed checkout's exact Git revision/source identity and release version; correlate with Herdr's `source.resolved_commit`/managed path where applicable. An invocation whose checkout no longer matches current registration must exit before mutation and ask the user to reopen the action. Display exact repository/revision, package version, retained roots, config/unit changes and possible build/download costs before consent. Do not build `main`, a moving tag or an unrelated latest release. Release selection and implemented package/CLI availability are recorded in the [status](../STATUS.md); a local foundation build is not a reviewed public release.
- Locally linked development is a separately disclosed source mode: explicitly consent to the exact local checkout/dirty-tree status and locked `path:` build, use isolated owned config/state/roots/units, and record local source identity rather than claim release provenance. It must not replace a production-owned artifact or impersonate a reviewed release.

### Retained build outputs, not profiles

The persistent owned installation directory is `$XDG_STATE_HOME/goatr/install` (default `~/.local/state/goatr/install`), outside `HERDR_PLUGIN_ROOT` and all Herdr-managed checkout generations. It contains a private installer lock/ownership record, stable per-revision Nix output roots, and `current`/`previous` activation pointers. It is installer bookkeeping, not another identity/config store. Enforce same-UID canonical private parents,0700 directories/0600 records and reject unsafe pre-existing entries; validate managed links against the recorded roots/store outputs rather than following arbitrary user links. Herdr plugin state paths confer no private-mode guarantee.

Use native `nix build --out-link` for a unique, final retained path such as `install/roots/<reviewed-commit>/result`, never a temporary checkout `result` or `--no-link`. The following is **future-only command shape**: `$release_rev` must first be a reviewed real full commit and `$retained_root` an exclusively owned final absolute root path; neither is a supplied release pin:

```sh
nix build --no-update-lock-file --out-link "$retained_root" "github:trevarj/goatr/$release_rev#goatr"
```

Require the expected single package output and executable; record any actual output suffix/path, resolved store path, source/lock/package provenance and root registration before activation. [Nix source/manual references](../REFERENCES.md#herdr-plugin-and-nix-retention-evidence) establish that local `nix build` output links register permanent indirect GC roots: the registered absolute link path must stay in place. **Never rename/move the retained result root during promotion.** Atomically replace only a separate `current` pointer in the same owned filesystem directory, pointing at the already-retained release output; keep the previous working output rooted. An arbitrary hand-made `current`/`previous` symlink is not itself proof of GC registration. Ordinary GC must retain active/previous/in-use closures even after Herdr checkout cleanup. Never use `nix profile install`, `nix-env`, `guix install`, `--profile`, the default/system profile or an unconsented global GC.

### Explicit install, failure and rerun

1. The pane presents the inspected source/operation plan and separate choices for install/update, existing setup and pairing. Decline/close before consent creates no package, service, key/config or pairing change. Acquire one same-user `flock` at `install/lock` before installer effects and hold it through build/setup/activation/removal; other panes/named servers report busy rather than race. Herdr's registration lock is not this lock. After obtaining it, re-read actual installation/source/ownership to detect another completed invocation. Direct setup also serializes its config writes and rejects intervening external changes.
2. Build at the final unique retained root while leaving current/previous pointers and any running service unchanged. Invoke the resulting **absolute** `bin/goatr setup` through its existing validation/explicit-consent flow; select configured wrapper-accessible runtime/project paths, not plugin cwd. Existing compatible installs are revalidated, not rebuilt or duplicated. Existing keys, host/project IDs, paired devices, config and receipts are preserved.
3. Setup preflights before committing configuration or units and separately offers an optional independently supervised user `serve` service. Declining service start leaves the rooted executable usable and prints its absolute `goatr serve` then `goatr pair` instructions. Once the user starts it and local health is confirmed, offer explicit `goatr pair` in this same local terminal. Pairing never starts serve or opens a pairing window without its own request; the host-wide authority disclosure remains unchanged.
4. SIGINT/pane closure terminates and waits for this invocation's build/setup children, releases the lock and reports cancelled, not installed/paired. Build/preflight/setup errors retain the previous working artifact/config/unit and give an actionable local error. A successful build alone is not setup success; installing without service consent is not a running/pairable claim. Do not detach build/serve from the pane.
5. Record source/version/root/store path, owned unit path/content identity and activation stage in a private atomically written record; no secrets. If interrupted between pointer/unit/record changes, rerun reconciles actual owned files/service executable before further consent, never trusts a success flag or guesses ownership. It may reuse a proven successful candidate root; failed candidates have no active pointer. Cleanup removes only proven failed, inactive, unused owned candidate roots, never a previous working or in-use artifact.

### Staged upgrades and independent lifetimes

A new plugin checkout is **not** an automatic companion update. Commit-pinned `herdr plugin update` refetches the same commit; moving to a reviewed new release requires explicit pinned reinstall followed by the installer action and new companion consent. Compare candidate release/version with the installed record and executable: an older/stale installer cannot silently downgrade; equal-version/different-source and unknown ordering require explicit source review, not guessed recency. Intentional rollback is a separate explicit decision, not normal install/rerun.

Build/validate the candidate and its setup compatibility before activation, retain old config/unit bytes and the old output root, then atomically activate the owned pointer. Upgrade may restart **only** its proven owned serve unit after specific consent; with restart declined the old daemon stays running and the pane reports the new artifact staged, not live. Health failure restores the prior pointer/owned serve unit and working executable; never regenerate identity or erase receipts. No destructive/incompatible config/state-format migration is committed by this installer: incompatibility blocks promotion until a separately reviewed migration/rollback path exists. Crash/failure at each transition must reconcile safely on rerun.

Unit `ExecStart` uses the absolute retained store-backed Rust executable and durable config/runtime paths, never checkout scripts/cwd. Herdr servers, OMP processes, Codex owners, TUIs and same-binary Codex relay units retain their independent lifetimes and exact executable bindings: no `PartOf`/stop propagation or implicit provider/relay restart on serve/plugin update. Keep every owned release root still referenced by those live units/processes, even if older than `previous`; no blanket pruning of old generations.

### Plugin uninstall versus owned companion removal

`herdr plugin disable`, `unlink` and `uninstall` remove/disable future plugin entrypoints only. There is **no manifest uninstall callback**; existing panes may continue, config/state is preserved and current upstream defers checkout cleanup. The independently installed Goatr executable/service/data remain available after plugin removal.

The pane offers a separately confirmed **Remove owned Goatr companion** operation before unregistering, with equivalent direct owned-resource instructions recorded for use after the plugin is gone. Under the installer lock, prove installation-record/path/unit-content/executable ownership, disclose effects, stop/remove only the owned serve unit, remove the owned activation pointers and release only unreferenced owned output roots. Do not delete store paths, run global GC, edit profiles, remove declarative/external units or stop Herdr/providers/owner/relay/TUI processes. Roots still serving an independent relay/provider lifetime remain retained and are reported as in-use, not falsely fully removed. Preserve pairing keys/devices, config, receipts and ownership/provenance records by default so reinstall can recover. Purging those durable data is another explicit local choice disclosing lost identity/pairings; never infer it from plugin uninstall.


## Configuration and trust roots

Version1 TOML at `$XDG_CONFIG_HOME/goatr/config.toml`, private0600. Example values are proposed Goatr configuration, not upstream Herdr fields. `/home/user` and `Workspace` are illustrative: replace them with the deployment user's home and wrapper-accessible workspace root; choose fresh project/owner UUIDs during setup. Provider-visible paths may differ from host paths and must be verified:

```toml
version = 1
runtime_dir = "/home/user/Workspace/.goatr"
default_codex_owner_id = "3d8c1c31-d88b-4924-9e08-d693d4cf6f64" # optional local creation selector
runtime_dir_agent = "/home/user/Workspace/.goatr"
allow_scoped_shell_probe = false # true only after explicit local scoped consent
herdr_sockets = [] # optional exact host-visible sockets, in addition to discovery
push_allowed_origins = ["https://ntfy.sh"]
omp_registry_dir = "/home/user/.omp/run/collab-hosts"
omp_session_roots = ["/home/user/.omp"]
# omp_blob_root is optional; setup fills only the verified active-profile blobs directory

[executables]
herdr = "/absolute/installed/herdr"
omp = "/absolute/installed/omp"
codex = "/absolute/installed/codex"

[[projects]]
id = "0f2da87e-5c07-4ed4-85fb-cf3d4dfb04e2"
label = "Goatr"
path = "/home/user/Workspace/goatr"

[[codex_owners]]
id = "3d8c1c31-d88b-4924-9e08-d693d4cf6f64"
socket_host = "/home/user/Workspace/.goatr/c.sock"
socket_agent = "/home/user/Workspace/.goatr/c.sock"
bindings_dir = "/home/user/Workspace/.goatr/b"
unit = "goatr-codex.service"
```

The example's OMP profile paths must be replaced with source-verified active-profile roots during setup; omission means discover the installed profile convention or report history unavailable, not guess a blob location. `executables.omp`/`codex` and corresponding backend-specific roots may be absent when that backend is unavailable; absence must not erase usable sessions of the other. `[executables]` paths are absolute existing installed wrapper expectations; Herdr uses its own executable directly for CLI discovery, Codex owner service uses the codex wrapper directly, but **Herdr kind-based TUI launch cannot take an executable path**. Its shell must demonstrably resolve the selected kind to that wrapper; see [launch preflight](HERDR_INTEGRATION.md#workspace-creation). Do not claim saving an absolute path controls `agent.start`.

Validate version/unknown keys/types, unique UUID project/owner IDs, canonical existing owner-private socket parents, allowed-origin syntax, usable project directories and control-free path/name fields. Expand `~` only during local setup and save absolute paths. Do not resolve wrappers to a raw underlying executable and bypass them. Setup `--project PATH` is repeatable; with none offer known Herdr workspace directories, never scan unrelated trees. Canonical projects must be accessible under the selected installed wrapper, not merely exist on the host.

Optional root `default_codex_owner_id` is one canonical UUID naming exactly one
configured `codex_owners` entry. Invalid/duplicate/unknown selectors fail
configuration validation and preflight. Omission selects the sole owner only
when exactly one exists; zero owners is setup-required, and multiple owners
without a selector disable creation precisely, never choose list order.
Existing-session attachment/control still considers **all** owners and is never
filtered by this selector or creation projects. Missing unselected owner paths
do not disable selected-owner creation; unsafe existing entries still fail trust
preflight. No project-owner mapping, wrapper/profile/environment knob, remote
selector, wire/UI method or service auto-adoption is added. Writable setup8 saves
an existing selected owner UUID only when needed, without treating it as proof.

Stock Codex0.159.1 owner paths are explicit advertised host/provider rendezvous
views, not a claim that `c.sock` itself is the physical socket. Its exact
same-UID alias target is
`/tmp/codex-daemon-<uid>/<SHA256(canonical advertised agent pathname)>`.
Configuration accepts only that source-backed shape under a canonical private
parent, or an explicit canonical private direct host mapping; this is trust and
attempt eligibility only. Herdr and helper-owned relay paths remain strict
nonsymlink sockets. Actual owner readiness additionally requires the original
owned service/native publisher/process/root/peer proof and private0700 physical
directory/socket0600 through the backend's held-root `openat2` resolver.
Do not follow a dangling host alias into another host owner, guess a PID, alter
the wrapper/TMPDIR/defaults, or auto-adopt a provisional Owner/Uncertain record.
Advertised host/provider and physical/descriptor connection paths retain the
107-byte guards. See [native owner source proof](CODEX_BACKEND.md#independent-owner-and-relay-services).

Goatr-created Codex TUIs alone receive the explicitly user-authorized stock
CLI prefix `-c check_for_update_on_startup=false` before the exact remote resume
arguments. This one-off client startup setting prevents the pre-connection
updater menu without an installation, persisted dismissal or global config
change. It is not applied to owner app-server launches, existing TUIs or OMP,
and adds no Goatr config/profile/wire setting. Other native defaults and
publisher/source checks remain unchanged; see [ordered Codex creation](CODEX_BACKEND.md#ordered-creation-and-authoritative-presence).

The separately authorized Codex zero-turn creation checkpoints are local fixed
behavior, not configuration or wrapper permission changes. After the original
paginated zero-turn `thread/start` acknowledgement and durable T retention
under Thread/Dispatching, make exactly one stock
`thread/name/set {threadId:T,name:<SessionCreateParams.name>}`.
This reuses the caller's already-requested display Label, not Herdr's private
correlation alias or a new title. Existing/failed threads are never renamed.
Only a correlated empty native
name-set result admits the one public
`thread/read {threadId:T,includeTurns:true}` on the original subscription/ticket.
Fresh authorization, live ticket/FD and unchanged owner/source/root identity
are required before both writes. Only exact same-T paginated empty-turn read
proof admits Relay/TUI creation. Legacy/unknown mode, title rejection/loss/
malformed result or read error fails closed without further write, model input,
retry, alternate thread, default/security/owner change or old uncertain-resource
adoption. The stock read deprecation notice remains enabled. Actual same-T TUI
resume and the desktop barrier must still be exercised with a new owned candidate.

Source contract, not a newly exercised Ready pass: the original Codex creation
readiness loop may wait only when its same-server `Topology::source` returns
`NotReady`. The existing 100-ms wait and 120-second deadline remain unchanged;
authorization/stopping/expiry are checked before each source lookup even while
unavailable. Recovery still requires all original manifest/source kernel,
incarnation, terminal, native-thread and root/binding evidence. Other errors
or changed identities retain uncertainty and fail closed, without relaunch,
title/read/resume replay, retargeting or a new timeout/configuration knob.





The user explicitly authorized a narrow exact-shell preflight exception:
Goatr may submit only fixed read-only resolution code through public
`pane.send_input`, in a fresh Goatr-created idle no-focus pane. It never probes
an existing pane or uses terminal input for native prompts/control. Private
bounded nonce/result artifacts and actual pane/shell/process/namespace/object
proof establish the selected outer wrapper before any provider launch.
Pre-workspace trust may admit only an empty owned workspace; it is not launch
authority or a borrowed old-pane measurement.
Input ACK means queued bytes, not completed evaluation; public Herdr has no
atomic foreground-epoch guard or queue recall. Reject detectable interference,
unknown delivery and late/replaced results, never resend or claim recall.
Shell history/hooks may observe the fixed query: the nonce is correlation,
not a credential, and no secrets/control payloads belong in it.
Setup retains explicit local scoped consent; this does not grant service start,
provider transition, global shell edits, focus changes or host-sandbox bypass.
Unsupported/unproved shell resolution remains creation-disabled.
Local TOML `allow_scoped_shell_probe` is boolean, omitted/default false.
Writable setup must disclose and obtain scoped consent before saving true;
it is not a remote/environment override, shell command or proof of resolution.
False disables only creation requiring this probe, never existing-session
inventory/read/control. Every new-root launch still needs actual matched proof.


**Paired authority covers all supported same-user host Herdr sessions.** The projects array limits only new creation; never filter inventory/history/control/notifications by it. Configuration and runtime paths cannot be changed remotely.

Three roots have separate ownership: durable keys/receipts/identity SQLite in `$XDG_STATE_HOME/goatr`; ephemeral local-admin socket at production default `$XDG_RUNTIME_DIR/goatr/admin.sock`; sandbox-visible provider integration files in configured `runtime_dir` (0700; for example `~/Workspace/.goatr`), seen by providers as `runtime_dir_agent`. Setup selects a wrapper-accessible integration directory rather than assuming a particular workspace layout. For an isolated local invocation, `serve`, `pair`, `devices` and `revoke` identically accept `--admin-socket PATH` under the [private-path and same-UID rules](PAIRING_SECURITY.md#exact-local-administration); no config key, environment override or remote socket-path setting is added. Setup/service installation keeps the production admin-socket default; smoke invokes serve directly and passes its owned socket path to every admin command, without changing XDG_RUNTIME_DIR. Only the explicit integration-root pair and exact owner socket pairs are supported for provider path mapping, not a generic mapping engine. Manifests record both provider paths; preflight proves they reference the same object. OMP overlay and Codex short socket/manifest paths derive from these integration roots. Production OMP relay port7466 is a runtime constructor default; owned tests inject a different loopback port and produce matching overlay.

## Platform and service ownership

Target Linux with Nix packaging and user-service support, including NixOS; no tested macOS/Windows installer claim. Preserve same-user environment, wrappers and signing policy. Herdr and shared Codex owner/relay services are independently supervised, never children killed on companion exit. Existing known Codex owners can be inspected, but a loaded thread without authoritative terminal binding is not an eligible live desktop session.

Deployment preflight must establish the installed wrapper's allowed launch directories, runtime-directory access, socket mappings and any host ADB/signing/SSH broker prerequisites. The originally inspected private wrapper required cwd under its workspace root, preserved `/run/user/<uid>` as XDG_RUNTIME_DIR, and mapped canonical same-user Herdr sockets from that runtime or `~/.config/herdr` to `/run/herdr-pi.sock`; these are **example deployment constraints, not universal Goatr APIs**. Validate the actual installed wrapper/version and report backend-specific incompatibility instead of opening host directories, relocating XDG_RUNTIME_DIR, weakening sandbox rules or bypassing signing. [Testing](../ACCEPTANCE.md#isolated-smoke-ownership) derives isolated paths from the verified deployment constraints.

Linux pathname Unix sockets must have UTF-8 paths ≤107 bytes, checked before bind/connect on **both** host and provider-visible paths. Use owner `c.sock`, relay `t<12 random hex>.sock` (exclusive pre-admission collision handling) and short owned smoke roots; full UUIDs belong inside manifests, not socket basenames. Fail with the exact oversized path locally; do not truncate a name or silently fall back to TCP.

The bounded foreground config owner loads version1 using retained
`toml_edit0.25.15+spec-1.1.0` parse/serde/display serialization, not handwritten TOML.
The actual local `setup` and ordinary `serve` accept `--config PATH`; omission
uses the private XDG default. Pair/list/revoke continue to use only the live
admin owner. Hidden isolated transport-smoke serve deliberately does not load
deployment configuration or start topology/providers.

`setup` is now a writable, local interactive-terminal workflow. An absent config
requires explicit `--runtime-dir` (an existing private wrapper-accessible
integration directory) and `--herdr` (the protected installed outer executable).
Optional `--omp`, `--codex`, `--omp-registry-dir`, repeatable
`--omp-session-root` and `--omp-blob-root` retain actual inspected deployment
choices; they never execute wrappers merely for versions. Provider-view runtime
may be supplied with `--runtime-dir-agent`. A fresh explicit shared owner uses
`--codex-owner-socket`, optional `--codex-owner-socket-agent`,
`--codex-bindings-dir` and optional `--codex-unit`; none starts Codex.
Existing configs preserve runtime/wrappers/owners instead of silently adopting
new flags. `--codex-owner UUID` selects an existing configured owner locally.

Repeatable `--project PATH` expands local `~`, canonicalizes and explicitly
approves same-UID protected directories; canonical repeats preserve IDs/labels.
Conflicting existing canonical IDs are refused, not discarded. With no choices,
setup only suggests current known Herdr cwd values, never scans unrelated trees
or automatically approves one. Static path/source trust and backend availability
are reported before consent; actual namespace views, new-root shell resolution
and native Ready remain guarded runtime proofs, not claims from saving a path.

`--allow-scoped-shell-probe` offers separate precise fresh-root-only consent
before saving true; omission remains false and `--disable-scoped-shell-probe`
explicitly removes it. Saving is another consent. Setup retains the expected
private opened FD/inode/timestamps/bytes, then holds an independent private
nonblocking `flock`, rechecks the exact source and publishes exclusive0600
same-parent temporary bytes with file/directory fsync and atomic no-replace
initial publication or proven-owned replacement. Intervening, managed,
foreign or symlink conflicts fail; identical reruns leave config bytes/inode
and IDs unchanged. No live HostIdentity/SQLite owner, rekey or receipt update is
opened by setup. Decline/cancel before consent causes no config/unit/key/provider
effect; cancellation after an admitted publication reports retained stages.

`--serve-unit` separately offers installation of the exact owned `goatr.service`,
then separately asks to start it. Ownership is private `.serve-unit.json` beside
the config plus exact protected unit content/source; same-name/manager/foreign
units or an unrelated default-admin owner are not adopted. A normal canonical
same-user non-writable unit directory may already be0755; it is not chmodded.
Unit exec/config/cwd and HOME/XDG roots are durable absolute local choices,
Type=exec/UMask0077/Restart=no, with no provider/Herdr/relay/TUI stop ties.
Before reload/start, setup rejects every unmanaged named/type-wide `.conf`
drop-in in the actual manager's `UnitPath`, every nonempty `DropInPaths`, a
foreign/masked/invalid fragment, pending daemon reload and backing content other
than the exact protected owned unit. It rechecks after daemon-reload, after the
human start consent and immediately at start admission, so an unrelated
overridden `ExecStart`/`ExecStartPre`/`ExecStartPost` cannot run merely to fail
the later health proof. No drop-in ownership or adoption is invented.

Readiness requires protected `devices.list` and exact unit PID/start/executable/
argv/cwd/socket-peer joins, never unit-active or `pair.open`. Already active
serve is not silently restarted/reloaded; changed config is reported staged.
Decline/unavailability prints absolute foreground serve, pair, devices and revoke
instructions. The package must be retained by its caller; final install-root/GC/
upgrade/removal/plugin policy is not implemented or implied by this basic unit.
Missing/unsafe projects remain unavailable creation choices and never erase
existing-session inventory. Executable validation retains the configured outer
wrapper/symlink, accepting appropriately protected root-owned immutable Nix
files rather than incorrectly imposing private-state-file permissions on them.
Provider-visible paths are validated syntactically and against measured object
views, never presumed accessible because the host spelling exists.
Disappearance of a configured optional provider wrapper/root or Codex owner
directory makes that backend/source unavailable, not the shared companion.
Read-only preflight still rejects unsafe existing entries, malformed paths and
untrusted shared roots; it does not blanket-ignore optional validation errors.
Setup and creation checks report the affected prerequisite without erasing the
other backend or retained inventory.
`Config::backend_reason(provider)` rechecks these optional prerequisites for
setup and each creation proof. Missing paths return a backend-specific reason;
unsafe existing paths remain errors. Configured paths and retained bindings are
not rewritten to hide the unavailable source.


Mapped ownership of immutable Nix store files is not private-state ownership.
Executable preflight may accept a canonical store object only with actual
kernel [`fstatvfs(ST_RDONLY)`](https://man7.org/linux/man-pages/man3/statvfs.3.html) read-only mount authority, while retaining strict trust checks for the
outer wrapper and ordinary mutable parents. A UID exception or a pathname
prefix alone is not authority.

## Acceptance gates

See [Herdr plugin lifecycle](../ACCEPTANCE.md#herdr-plugin-installation-and-lifecycle), [checkpoint commands](../ACCEPTANCE.md#implemented-checkpoint-commands-and-final-proposed-owners), [isolated smoke ownership](../ACCEPTANCE.md#isolated-smoke-ownership), [desktop-visible creation](../ACCEPTANCE.md#live-session-and-creation), [companion restart](../ACCEPTANCE.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.
