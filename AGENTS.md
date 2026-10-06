# Repository instructions

## Authority and current boundary

Read `plans/initial/GOATR_PLAN.md`, your assigned topic, its dependencies, and
`plans/initial/REFERENCES.md` before editing. Those documents own product
contracts; README describes the infrastructure currently present. This checkout
is a foundation, not completed package 1A. Do not invent product commands,
dummy bindings, insecure native fallbacks, or passing runtime evidence.

## One coordinator, isolated writers

- One named integration owner controls each work package and its merge/check
  handoff. Workers **must not recursively delegate**; only the coordinating
  developer/agent assigns independent slices.
- Give every concurrent writer its own branch and Git worktree, for example
  `git worktree add -b feature/<slice> ../goatr-<slice>`. Inspect existing branches
  and paths first; never overwrite another worktree or another developer's work.
- The coordinator records path ownership and shared-contract owners in the task
  assignment before work begins. A file has one writer. Independent worktrees
  do not grant permission to change another owner's files/contracts.
- Message the owner before touching a shared file. Schema/config/identity/native
  manifest changes land through their owner, with every consumer and matching
  fixture updated in the same handoff. Do not add aliases or parallel definitions
  to avoid coordinating a clean cutover.
- Commits, pushes, paid signup, service/global-tool installation, release keys,
  private-source copying, production resources, and physical-device use require
  explicit authorization. The project-scoped SDK consent in the optional Nix
  shells is already authorized; it is not permission to change global settings.

## Shared ownership map

These are phase ownership boundaries, not claims that future files exist.

| Owner | Paths/contracts |
|---|---|
| Foundation / 1A native-build | `flake.nix`, `flake.lock`, `.envrc`, `pyproject.toml`, `Makefile`, `.github/`, root notices; future Gradle root/wrapper, `third_party/iroh-ffi/`, `iroh-jvm/`, `iroh-android/`, native build/check tooling and artifact manifest |
| 2A protocol | `protocol/`, canonical wire schema/types/hash fixtures and both Python/Kotlin protocol consumers; owns every shared wire name |
| 2B state/storage | Serialized reducers, generation/receipt/content/cache/read state and finite retention; coordinate shared Android/companion models with 2A |
| 3 security | Pairing/admin/authorization/revocation contracts and key-store lifecycle; CLI arguments must match packaging contracts |
| 4 topology/config | Configuration/trust roots, runtime-path/wrapper resolution and Herdr session identity/creation contracts |
| 5A OMP / 5B Codex | Separate provider-specific adapter paths only after 2–4 inputs are frozen |
| 6 Android facade | `app/` UI/lifecycle, excluding shared wire/state/native ownership above |
| 7 delivery | Foreground/UnifiedPush delivery seams, attention ownership shared with 2B and revocation with 3 |
| 8 integration/packaging | Final executable/setup/service packaging, owned smoke runners and integrated evidence |

A topic edit belongs to its contract owner. The coordinator owns cross-topic
plan status, README/AGENTS updates, and resolving overlapping assignments.

## Development and handoff

Use the locked shell, not global pip/cargo/rustup/SDK installs. `.envrc` is
`use flake`; use `nix develop path:.` while foundation files remain untracked.
`make lint`, `make format`, `make format-check`, `make smoke`, and `make check`
are shared local/CI entrypoints. Add only a runnable check that exercises actual
new behavior; the dependency smoke is not a transport or product test.

Workers do not run builds, tests, linters, formatters, or smoke checks mid-flight
unless verification is their explicit assignment. After all owned changes land,
one integration owner runs the relevant checks once, including:

```sh
nix develop -c make check
nix build .#companion --no-link
```

For optional Android tool configuration, evaluate the `android` and `emulator`
shell derivation paths before downloading/building their closures. Follow
`plans/initial/TESTING.md` for later native/product gates, with owned resources.
Do not call missing future commands to manufacture a pass or silently skip a
selected backend. Never log secrets or touch another user's sessions/state.

Every handoff names changed paths, exact executed commands/results (or clearly
unexecuted coordinator checks), source/lock identity, artifacts, actual behavior
and precise blockers. No build/runtime/advisory/16-KiB claim without its own
observed evidence. Source/URL availability alone is not compatibility evidence.

## Next gates

1. Finish 1A: retained secure FFI migration/lock/patches; same manifest and full
   API for Python/Kotlin; both Android ABIs and host output; advisory, notices,
   JNA inventory, static alignment and owned native-load evidence.
2. Freeze 2A types/method table/canonical fixtures before adapters or dispatch;
   then implement 2B state and phase 3 authorization/revocation.
3. Complete real framed selected direct/relay action gates after 2–3. Provider,
   UI, background and full integrated gates remain later phases, not foundation
   acceptance. Keep plan verification status scoped to what actually ran.
