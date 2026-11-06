# Goatr design studio

A local review gate for Heeler reference screenshots, annotated Android wireframes, feature scope and implementation decisions. **Design review — application rewrite not implemented.** References, wireframes and design acceptance are not evidence of production behavior, runtime parity or a completed Android rewrite.

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
