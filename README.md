# Goatr

Goatr is an independent Android Herdr client based on Zagal, with Goatr's branded
assets. Connect through paired, end-to-end encrypted **iroh**, or **SSH** with
pinned host-key trust. There is no Goatr account or hosted Goatr service.

This is development source, not a supported release. You need **Nix on x86_64
Linux**, this checkout, and an **Android 10+** arm64-v8a or x86_64 phone/tablet.

## Install on Android

Enable Android **Developer options → USB debugging**, connect your device by USB,
and authorize this computer on the device. From the checkout:

```sh
nix run .#install-debug
```

This builds and installs on the single authorized physical USB device. To select a
particular physical device (including one already connected through wireless ADB):

```sh
nix run .#install-debug -- --serial SERIAL
```

Emulators are refused. The selected serial stays pinned throughout the build.
Installation updates the same app without clearing its data; it never uninstalls
an app or forces a downgrade. Debug uses `io.github.trevarj.goatr.dev`, alongside
release Goatr (`io.github.trevarj.goatr`) and the older `.debug` app. There is no
automatic data migration between these app IDs.

For a release install, supply your **own existing release keystore**:

```sh
nix run .#install-release -- --keystore /path/to/release.keystore
# Optional: --key-alias ALIAS and/or --serial SERIAL
```

`apksigner` prompts for passwords interactively; release installation requires
terminal stdin and stdout (no pipes or redirected input/output). No release keys
are included or generated. Use the same signing key as the installed release to preserve updates;
a signature mismatch fails without uninstalling anything. The unsigned APK is
retained, and the separately signed APK is
`app/build/outputs/apk/release/app-release.apk`. Both installers verify the APK
signature before installation. Use `-- --help` for usage without building or
accessing a device.

Build only, without installing: `nix run .#apk-debug` or
`nix run .#apk-release-unsigned` (the latter still needs distributor signing).
Developer commands, exact pins and validation guidance are in [AGENTS.md](AGENTS.md).

## Start the desktop and pair

Use compatible **Herdr Core 0.1.19** (`hc connect`), not the older incompatible
Herdr 0.9.1 workspace manager. The project packages the unchanged official `hc`
without replacing any installed Herdr binary. If you do not already have an
intended compatible core, run these in three desktop terminals:

```sh
# Terminal 1: start a new core
nix run .#hc -- serve

# Terminal 2: start the full-control bridge
nix run .#desktop

# Terminal 3: create a pairing invitation
nix run .#goatr -- pair
```

Use the same `HC_SOCKET`/`HC_SESSION` values for the intended compatible core and
bridge when set. Do not adopt existing desktop sessions during checks. Startup
status appears on stderr; “Listening” means local listeners are bound, not that
Herdr Core or relay/discovery is ready. Add `-L` (for example,
`nix run -L .#desktop`) to see Nix build/compiler logs.

On Android, open **Settings → Paired desktops (iroh) → Pair a desktop**. Scan the
QR or **Paste** the invitation URI printed above it. The QR appears only when the
whole code fits: fullscreen/enlarge the terminal or zoom out and rerun `pair`, or
paste the URI instead. Compare the **complete desktop fingerprint**, read the
full-control authority, and explicitly **Confirm full control**. Invitations
expire in five minutes and are single-use; preview alone never pairs. Select the
paired desktop to connect. Reachable machines are listed separately, and changing
desktops never silently retargets an action. SSH hosts can stay connected alongside.

Pairing grants **full same-user Herdr control**: terminal commands, uploads,
history, notifications, and closing tabs/spaces on every reachable machine. Old
pairings require a fresh invitation for HC access. List phones with
`nix run .#goatr -- devices`; revoke one with
`nix run .#goatr -- revoke DEVICE_ID`. Removing a desktop on the phone is **not**
desktop-side revocation. Disconnecting does not kill desktop jobs.

In **Settings → Home relay region**, select **Automatic** (recommended) or a
built-in region, then **Save and reconnect**. This replaces the phone's iroh
endpoint; SSH is unchanged. A fixed region restricts home-relay candidates without
soft fallback, not all traffic geographically: direct paths and the desktop's
relay remain possible. Discovery/relays see connection metadata, not plaintext
terminal traffic.

For SSH, add `user@host[:port]` under **SSH hosts**, independently verify the host
fingerprint, and follow the protected phone-key setup. The host needs compatible
`hc connect` on its SSH command path. Agent notifications are opt-in and subject
to Android background/network limits; the replaced app's ntfy/UnifiedPush interface
is not part of this Android product.

## License and provenance

Independent Goatr code is **GPL-3.0-or-later** ([LICENSE](LICENSE)). Imported
Zagal code is Copyright 2026 Osc100, **Apache-2.0** ([ZAGAL-LICENSE](ZAGAL-LICENSE)),
source revision `1e73cd24d2abc049c5859a2865c5fa4659a0f5b6`.
See [NOTICE](NOTICE) and [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for
upstream/native/font/icon/sound notices. Herdr and other upstream marks retain
their owners' trademark rights.
