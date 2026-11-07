# Goatr

Goatr is an independent Android app and Linux companion under development.
The planned rewrite is a Heeler-composition native Android client for
**existing herdr-core sessions over paired iroh**; the legacy implementation
has not been replaced.

## Availability

Goatr is under development and not yet ready for use. No supported release or
installation workflow is available; installation and pairing instructions will
be provided when the product is ready.

## Planned core-only rewrite — design review, not implemented

The [rewrite contract](HEELER_ANDROID_REWRITE_PLAN.md) and
[local design studio](studio/README.md) describe the target; production work
waits for studio review and a later implementation handoff.

- Android speaks the native core protocol through an authenticated, authorized
  iroh byte bridge to an exact host-configured, already-running Linux core socket.
  No private Goatr core, new SSH/public TCP, companion PTY or second terminal API.
- Adopt existing sessions/terminals created by any hc client, without mandatory
  Goatr labels/reporters. Native clients can view the same terminals concurrently;
  last-active shared sizing applies, and mobile disconnect preserves programs,
  metadata and other clients.
- Genuine pairing previews full core-account command/file/terminal and configured
  machine access—not a per-terminal sandbox. No automatic replay of uncertain
  input, runs, kills or writes.
- Preserve the full product outcomes, including five structured provider Chats,
  Accounts, creation/worktrees, read-only Changes, guarded config Undo, private
  atomic staging up to 64 MiB, encrypted disconnected ntfy/UnifiedPush, Usage and
  adaptive terminal UI. Native primitives do not supply all these workflows;
  precise staging/config/provider/Host-service gaps remain explicit in the contract.

Heeler's visual reference stays frozen at `17e890481ae5a80e142395ab17974cee8655c6a4`.
Core source `9ce385d9c8abb041dd4437476d770365b3fb5282` (declared `0.1.18`,
generation `1`) is static research only, **not an installed/runtime-qualified
version choice**. This review makes no implementation, installation or live-provider
claim; OpenCode structured Chat is reference-unavailable, not terminal-unavailable.

## Legacy design scope — unchanged implementation sources

The following describes the older product direction, not the core-only rewrite
contract or a supported working release:

- Pair a phone with a desktop by scanning a QR code, then connect directly or
  through a public relay without setting up DNS, port forwarding, SSH, Termux,
  or a VPN.
- In **Settings → Home relay region**, leave **Automatic** (recommended) or
  choose a region, then explicitly **Save and reconnect**; all phone connections
  briefly disconnect. A fixed region restricts this phone's home-relay candidates,
  with no soft fallback to other regions. It does not confine all traffic
  geographically: direct connections and a desktop's own relay remain available.
- Share existing same-user Herdr sessions and create new sessions without
  taking over the desktop.
- Read conversations and tool activity, send prompts, cancel work, and respond
  to dialogs exposed by the stock OMP and Codex protocols.
- Choose persistent connectivity or ntfy via UnifiedPush for background
  notifications.

Only stock OMP and Codex are intended to be supported, not Pi or patched agent
runtimes. Unsupported or custom dialogs remain desktop-only. Goatr is a native
mobile interface, not a terminal emulator, and does not require a hosted Goatr
account. Background delivery remains subject to Android and network limits.

## License and provenance

Independent Goatr code is **GPL-3.0-or-later**; see [LICENSE](LICENSE) and
[NOTICE](NOTICE). No private/reference application source was copied. Retain
applicable upstream notices before importing or distributing third-party code.
