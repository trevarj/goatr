# Paging handoff

The paging worktree is integrated into this checkout. [STATUS.md](STATUS.md)
owns current artifacts, observed results and next actions; [acceptance](ACCEPTANCE.md)
owns runner commands and gates. There is no separate paging writer to wait for.

## Current behavior

- Protocol v2 schema, fixtures and Rust/Kotlin consumers moved together.
- Native browse views follow exact provider cursors beyond the live1000-row window.
- Android preserves successors while retaining one bounded1000-record display/cache window.
- Repeated captured pages remain stable; source, peer, generation, expiry and resource fences remain required.

The owner approved extending traversal beyond1000 records. Core, native fixture
and reducer changes are integrated/reviewed; all-target Rust tests and matching
Android builds/checks passed. These are component evidence, not live proof.
The strict runner uses1024 independent completions plus one witnessed append per
provider, requires an older ID outside the frozen consumer window with at least1000
later completions, and checks real Android display and return to latest.
Latest strict attempt passed OMP creation and prompt/Stop, then stopped because
16 independently completed commands took101038ms, exceeding the one-hour
projection for1024. Android feedback proceeds with this costly boundary proof
explicitly deferred. No paging pass is claimed.

## Retained integration provenance

Source worktree: /home/trev/Workspace/goatr-paging-proof, feature/paging-proof-sol61;
HEAD a55cd979cab6a0a47d4c757ac97b97d8535ee9eb. The filesystem three-way integration
preserved this checkout's native/Android changes without resetting Git.
Original106-file manifest SHA256:01512efa5f60a7a6818fb802f81c0748839cb6cf832f38b47504ba32b4964527.
Reconciled36-path selection SHA256:ef28e4738772c0a5be710f51eecda5797c5e0f62e3962495378146dd9b40a311.
Immutable handoff copies: /home/trev/Workspace/.gs-paging-owner-final-2VgS0G;
main preimages: /home/trev/Workspace/.gs-v2-main-integration-sDzfU7.
Historical both-provider Android continuity passed with strict paging deferred;
its expired endpoint is not a current connection instruction.
