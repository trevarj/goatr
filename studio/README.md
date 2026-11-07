# Goatr design studio

A local review gate for Heeler reference screenshots, annotated Android wireframes, feature scope and implementation decisions. **Design review — application rewrite not implemented.** References, wireframes and design acceptance are not evidence of production behavior, runtime parity or a completed Android rewrite.

## Core-only review target

Preserve the Heeler visual/behavior reference at
[`17e890481ae5a80e142395ab17974cee8655c6a4`](https://github.com/ZingerLittleBee/Heeler/tree/17e890481ae5a80e142395ab17974cee8655c6a4),
the static Material artwork and all scene/variant coverage. Design revision **5**
pivots active scope to Android native core → authorized paired iroh stream → exact
host-configured **existing** Linux core client socket, not a second terminal protocol
or companion PTY/runtime. No `hc connect` autostart, new SSH/public TCP or arbitrary
mobile-selected socket destination.

Goatr must adopt existing sessions/terminals from **any native hc client**, without
private Goatr core, ownership-only filtering, mandatory labels/reporters or metadata
rewrites. Other native clients may attach concurrently; last-active shared sizing
applies. Mobile detach/disconnect preserves programs, session and other clients.
Pairing must disclose full core-account command/file/terminal/server and configured
machine authority, authorize before socket forwarding, and revoke actual peer
streams plus corresponding socket handles. It is not a terminal/Workspace sandbox.

The separate core research pin is
[`9ce385d9c8abb041dd4437476d770365b3fb5282`](https://github.com/herdrdev/herdr-core/tree/9ce385d9c8abb041dd4437476d770365b3fb5282)
(declared source `0.1.18`, generation `1`): **not an installed/runtime-qualified
version choice**. Native capabilities are not finished product workflows. Five core
structured Chats, Accounts, directory/Workspace adoption, creation/worktrees,
read-only Changes, config Undo, encrypted background delivery and Usage stay required.
OpenCode structured Chat remains reference-unavailable; real OpenCode terminals are
not excluded. TUI-only reference operations are explicitly excluded historical rows;
independent Apply to All layout copying stays active.

Visible unresolved implementation work includes provider/process/transcript/binary
file discovery; private **atomic 64 MiB staging** versus native `terminal.put`'s
**8 MiB** direct-write limit; guarded expected-bytes/nonexistence config writes/Undo
absent from stock `file.write/edit`; and a separate supporting Host service for
encrypted ntfy/UnifiedPush while Android is disconnected. Core live events and a
transparent tunnel do not deliver offline push or supply Usage/account/Git APIs.
Use actual native file/`machine.exec`/terminal/template primitives, not invented
workspace/tab/account/template-CRUD or extra companion APIs. Fresh reconnect
snapshot/sequence/channels/core identity/documents never replay uncertain effects;
JSON-RPC IDs are not durable idempotency receipts.

See [the full rewrite contract](../HEELER_ANDROID_REWRITE_PLAN.md) for precise
capability boundaries and future acceptance. No production code, installation,
service replacement, provider call or runtime qualification is part of this review.

## Start

From the checkout root, with Nix installed:

```sh
nix develop -c make studio
```

Open **http://127.0.0.1:4173/**. The target runs `darkhttpd studio --addr 127.0.0.1 --port 4173`: only `studio/` is served, on loopback only. Press **Ctrl-C** in the serving terminal to stop it.

If that port is occupied, leave its owner running. From the checkout root, enter the Nix shell and manually choose a free port, for example:

```sh
nix develop
darkhttpd studio --addr 127.0.0.1 --port 4174
```

Then open http://127.0.0.1:4174/. Never serve the repository root or bind publicly.

Use the Git-backed commands above for this committed checkout: Nix excludes ignored build outputs. Only use `path:.` when new, untracked development-shell inputs must be included; it can copy ignored outputs too. Preserve existing caches rather than deleting them to make room.

## Review

- Search/select a screen, form factor and theme; the URL hash preserves the selection. Toggle annotations and inspect feature/source evidence.
- Use side-by-side, overlay or difference views and optional local image pickers/crops. Numeric comparison requires equal canvas dimensions; it is diagnostic, not a production parity pass.
- Save a review state and notes per screen/variant, then export review JSON. Notes are browser-local, scoped to the origin and reference revision, not shared or server-backed. If storage is unavailable, notes remain in memory only: export before leaving. Changing ports uses a different browser origin.
- Increment `catalog.json`'s `designRevision` when design content, geometry, variants, behavior annotations or associated references change. Previous notes remain, but acceptance becomes stale and needs explicit renewal for the new revision.

To run the browser checks, open the studio's developer console:

```js
await import('/checks.js').then(m=>m.run())
```

## Local assets

The studio uses static HTML/CSS/JavaScript, local reference assets, vendored Pixelmatch and filled Material icon SVG paths—not Material UI components. No npm install, CDN, provider accounts or provider calls are required. Source links are citations, not runtime dependencies; local image selections stay in the browser. Icon provenance and licenses are recorded in `vendor/material-icons-NOTICE.txt` and the studio license inventory.
