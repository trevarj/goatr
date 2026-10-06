# Linux companion packaging and setup

Scope: explicit human-invoked setup, CLI/Nix packaging, canonical local configuration and provider-preserving process/service ownership. Read configuration/trust roots before implementing discovery or launch.

[Main plan](GOATR_PLAN.md) · [Tech stack](TECH_STACK.md) · [Pairing and administration](PAIRING_SECURITY.md) · [Herdr integration](HERDR_INTEGRATION.md) · [OMP launch](OMP_BACKEND.md#local-relay-and-new-session-launch) · [Codex services](CODEX_BACKEND.md#independent-owner-and-relay-services)

## Packaging and explicit setup

Provide `goatr serve`, `goatr setup`, pairing/revocation commands and Nix `packages.goatr`/`apps.goatr`. Human-invoked setup validates installed Herdr/OMP/Codex compatibility, configured paths and wrapper launch resolution before offering optional user-service installation/start. Declining service startup prints `goatr serve` then `goatr pair` instructions; pairing does not silently start it. Setup never installs/replaces providers, signs up for infrastructure, kills sessions or relaunches embedded Codex. Re-run validates and preserves identity/project IDs and identical owned files; changed configuration is shown for explicit acceptance. Managed/symlinked config/unit conflicts fail rather than overwrite. Production setup is not authorized by this plan.

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

See [proposed command suite](TESTING.md#proposed-command-suite), [isolated smoke ownership](TESTING.md#isolated-smoke-ownership), [desktop-visible creation](TESTING.md#live-session-and-creation), [companion restart](TESTING.md#mutation-receipts-and-companion-restart). These are required implementation checks, not recorded passing results.
