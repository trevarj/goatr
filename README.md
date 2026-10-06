# Goatr

Goatr is an independent Android app and Linux companion being built for mobile
access to desktop Herdr sessions running stock OMP or Codex.

## Availability

Goatr is not yet available to install or use. There is no runnable companion,
Android app, or release artifact; installation and pairing instructions will be
provided when the product is ready.

## Intended scope

- Pair a phone with a desktop by scanning a QR code, then connect directly or
  through a public relay without setting up DNS, port forwarding, SSH, Termux,
  or a VPN.
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
