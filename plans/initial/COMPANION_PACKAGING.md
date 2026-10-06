# Linux companion packaging and setup

Scope: explicit human-invoked setup, CLI/Nix packaging, canonical local configuration and provider-preserving process/service ownership. Read configuration/trust roots before implementing discovery or launch.

[Main plan](GOATR_PLAN.md) · [Tech stack](TECH_STACK.md) · [Pairing and administration](PAIRING_SECURITY.md) · [Herdr integration](HERDR_INTEGRATION.md) · [OMP launch](OMP_BACKEND.md#local-relay-and-new-session-launch) · [Codex services](CODEX_BACKEND.md#independent-owner-and-relay-services)

## Packaging and explicit setup

Provide `goatr serve`, `goatr setup`, pairing/revocation commands and Nix `packages.goatr`/`apps.goatr` from the single Rust package/retained lock in [Tech stack](TECH_STACK.md#companion-rust-package-and-dependency-resolution). The same packaged binary has an internal `codex-relay --binding PATH` subcommand, invoked as its own independently supervised service, never tied to `serve` lifetime. Human-invoked setup validates installed Herdr/OMP/Codex compatibility, configured paths and wrapper launch resolution before offering optional user-service installation/start. Declining service startup prints `goatr serve` then `goatr pair` instructions; pairing does not silently start it. Setup never installs/replaces providers, signs up for infrastructure, kills sessions or relaunches embedded Codex. Re-run validates and preserves identity/project IDs and identical owned files; changed configuration is shown for explicit acceptance. Managed/symlinked config/unit conflicts fail rather than overwrite. Production setup is not authorized by this plan.

## Herdr plugin install and companion lifecycle

Phase8 provides the optional Linux [plugin-v1 entrypoint](HERDR_INTEGRATION.md#supported-plugin-v1-entrypoint): `plugins/goatr/herdr-plugin.toml` and one executable Bash dispatcher `goatr.sh`. This is a convenience front door to the same Rust package/setup/pair flows, not a second installer daemon, SDK or Cargo package. Plugin link/install/enable/Herdr startup never builds, installs, starts or pairs the companion. Only the explicitly opened interactive installer pane offers those operations.

### Source selection and preflight

- Inspect an existing `goatr` and its service/config ownership first. An externally installed or declaratively managed binary/unit/config remains externally owned: offer its compatible absolute executable for existing setup/pair, or report the exact manager-owned change needed. Missing Nix does not prevent using that compatible installation. Never shadow, adopt, replace or remove it merely because its name matches. Conflicting/unproven ownership blocks owned installation; no automatic second daemon/admin socket.
- For an owned install, check Linux/architecture, actual deployed Herdr plugin API, Bash/Git/`flock`, and a supported Nix command/version contract with `nix-command`/`flakes`, local filesystem store/daemon permissions, network and private writable installation roots. Nix2.28 documentation is inspected evidence, not a selected/tested minimum. Remote-only stores cannot satisfy local executable/output-root retention. Missing prerequisites produce visible diagnosis and manual/declarative guidance, never `sudo`, `curl|sh`, global tool installation, Nix configuration/trusted-key/cache changes or provider mutations.
- Production source is the public `trevarj/goatr` repository at a **reviewed immutable release commit**, with the retained flake/Cargo locks and `packages.goatr` build output. Obtain and validate the managed checkout's exact Git revision/source identity and release version; correlate with Herdr's `source.resolved_commit`/managed path where applicable. An invocation whose checkout no longer matches current registration must exit before mutation and ask the user to reopen the action. Display exact repository/revision, package version, retained roots, config/unit changes and possible build/download costs before consent. Do not build `main`, a moving tag or an unrelated latest release. No approved Goatr release revision or `packages.goatr` exists yet.
- Locally linked development is a separately disclosed source mode: explicitly consent to the exact local checkout/dirty-tree status and locked `path:` build, use isolated owned config/state/roots/units, and record local source identity rather than claim release provenance. It must not replace a production-owned artifact or impersonate a reviewed release.

### Retained build outputs, not profiles

The persistent owned installation directory is `$XDG_STATE_HOME/goatr/install` (default `~/.local/state/goatr/install`), outside `HERDR_PLUGIN_ROOT` and all Herdr-managed checkout generations. It contains a private installer lock/ownership record, stable per-revision Nix output roots, and `current`/`previous` activation pointers. It is installer bookkeeping, not another identity/config store. Enforce same-UID canonical private parents,0700 directories/0600 records and reject unsafe pre-existing entries; validate managed links against the recorded roots/store outputs rather than following arbitrary user links. Herdr plugin state paths confer no private-mode guarantee.

Use native `nix build --out-link` for a unique, final retained path such as `install/roots/<reviewed-commit>/result`, never a temporary checkout `result` or `--no-link`. The following is **future-only command shape**: `$release_rev` must first be a reviewed real full commit and `$retained_root` an exclusively owned final absolute root path; neither is a supplied release pin:

```sh
nix build --no-update-lock-file --out-link "$retained_root" "github:trevarj/goatr/$release_rev#goatr"
```

Require the expected single package output and executable; record any actual output suffix/path, resolved store path, source/lock/package provenance and root registration before activation. [Nix source/manual references](REFERENCES.md#herdr-plugin-and-nix-retention-evidence) establish that local `nix build` output links register permanent indirect GC roots: the registered absolute link path must stay in place. **Never rename/move the retained result root during promotion.** Atomically replace only a separate `current` pointer in the same owned filesystem directory, pointing at the already-retained release output; keep the previous working output rooted. An arbitrary hand-made `current`/`previous` symlink is not itself proof of GC registration. Ordinary GC must retain active/previous/in-use closures even after Herdr checkout cleanup. Never use `nix profile install`, `nix-env`, `guix install`, `--profile`, the default/system profile or an unconsented global GC.

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
runtime_dir_agent = "/home/user/Workspace/.goatr"
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

**Paired authority covers all supported same-user host Herdr sessions.** The projects array limits only new creation; never filter inventory/history/control/notifications by it. Configuration and runtime paths cannot be changed remotely.

Three roots have separate ownership: durable keys/receipts/identity SQLite in `$XDG_STATE_HOME/goatr`; ephemeral local-admin socket at production default `$XDG_RUNTIME_DIR/goatr/admin.sock`; sandbox-visible provider integration files in configured `runtime_dir` (0700; for example `~/Workspace/.goatr`), seen by providers as `runtime_dir_agent`. Setup selects a wrapper-accessible integration directory rather than assuming a particular workspace layout. For an isolated local invocation, `serve`, `pair`, `devices` and `revoke` identically accept `--admin-socket PATH` under the [private-path and same-UID rules](PAIRING_SECURITY.md#exact-local-administration); no config key, environment override or remote socket-path setting is added. Setup/service installation keeps the production admin-socket default; smoke invokes serve directly and passes its owned socket path to every admin command, without changing XDG_RUNTIME_DIR. Only the explicit integration-root pair and exact owner socket pairs are supported for provider path mapping, not a generic mapping engine. Manifests record both provider paths; preflight proves they reference the same object. OMP overlay and Codex short socket/manifest paths derive from these integration roots. Production OMP relay port7466 is a runtime constructor default; owned tests inject a different loopback port and produce matching overlay.

## Platform and service ownership

Target Linux with Nix packaging and user-service support, including NixOS; no tested macOS/Windows installer claim. Preserve same-user environment, wrappers and signing policy. Herdr and shared Codex owner/relay services are independently supervised, never children killed on companion exit. Existing known Codex owners can be inspected, but a loaded thread without authoritative terminal binding is not an eligible live desktop session.

Deployment preflight must establish the installed wrapper's allowed launch directories, runtime-directory access, socket mappings and any host ADB/signing/SSH broker prerequisites. The originally inspected private wrapper required cwd under its workspace root, preserved `/run/user/<uid>` as XDG_RUNTIME_DIR, and mapped canonical same-user Herdr sockets from that runtime or `~/.config/herdr` to `/run/herdr-pi.sock`; these are **example deployment constraints, not universal Goatr APIs**. Validate the actual installed wrapper/version and report backend-specific incompatibility instead of opening host directories, relocating XDG_RUNTIME_DIR, weakening sandbox rules or bypassing signing. [Testing](TESTING.md#isolated-smoke-ownership) derives isolated paths from the verified deployment constraints.

Linux pathname Unix sockets must have UTF-8 paths ≤107 bytes, checked before bind/connect on **both** host and provider-visible paths. Use owner `c.sock`, relay `t-<12 random hex>.sock` (exclusive creation/collision retry) and short owned smoke roots; full UUIDs belong inside manifests, not socket basenames. Fail with the exact oversized path locally; do not truncate a name or silently fall back to TCP.

## Acceptance gates

See [Herdr plugin lifecycle](TESTING.md#herdr-plugin-installation-and-lifecycle), [proposed command suite](TESTING.md#proposed-command-suite), [isolated smoke ownership](TESTING.md#isolated-smoke-ownership), [desktop-visible creation](TESTING.md#live-session-and-creation), [companion restart](TESTING.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.
