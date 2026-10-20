# Keeping the fork in step with upstream

This fork (`navraj007in/soundcraft`) tracks `storytold/soundcraft` ("upstream"), which moves fast: about 375
commits in its first three days after the fork point, then another 138 in one merge burst a week later.
The fork's own work is the SoundCraft-AI layer and the UI built for it (see `docs/soundcraft-ai/`).
Upstream builds the essential DAW. This page is how we carry the first while taking the second.

## Setup (once per clone)

```
git remote add upstream https://github.com/storytold/soundcraft.git   # if missing
git config rerere.enabled true      # remember conflict resolutions and replay them next time
scripts/upstream-sync.sh status     # creates the `upstream-main` mirror branch
```

`upstream-main` is a **mirror**: it only ever fast-forwards to `upstream/main`. Never commit to it. Then
`git log upstream-main..main` is exactly what the fork carries, and `git log main..upstream-main` is what
it lacks.

## The weekly routine

1. `scripts/upstream-sync.sh status`: what is new upstream, and which of those files we also changed.
2. `scripts/upstream-sync.sh start`: updates the mirror, branches `sync/upstream-YYYYMMDD` from `main`,
   merges upstream, and lists any conflicts. It never pushes.
3. Resolve conflicts (below), `git add`, `git commit`, then run `cargo xtask ci`.
4. Merge the sync branch into `main` and push. One merge per sync, not one per upstream pull request.

Sync **weekly**, or at an upstream release. The size of a merge grows with the gap: the first one touched
21 conflicting files, a later one 10, and both times several of our fixes turned out to duplicate upstream's.

## Resolving conflicts

- **Where both sides fixed the same problem, keep upstream's version** and keep our regression test on
  top if it still passes. Carrying a second implementation of something upstream owns only makes the next
  merge harder.
- **Keep ours where the fork adds something upstream does not have**: the AI layer, the control-channel
  authentication, the realtime worker pool (`crates/rt-pool`), the design system palette and fonts.
- **Docs headers and revision tables** (`ROADMAP.md`, `docs/gaps.md`, `docs/roadmap.md`, the parity pages)
  conflict on every sync. Take the newest date, join the change notes, and keep both sets of revision rows,
  newest first.
- After resolving: `cargo xtask ci`. A merge that builds can still break a test that asserted the old
  shared behavior; fix the test to match upstream, not the other way round.

## Where conflicts come from (hot files)

These are the files both sides edit most. Avoid editing them for fork-only work, and keep any edit small:

| Area | Files |
|---|---|
| Mix and playback | `crates/mix/src/lib.rs`, `crates/playback/src/lib.rs`, `crates/playback/src/record.rs` |
| Engine | `crates/engine/src/lib.rs`, `crates/engine/src/io.rs` |
| Time and model | `crates/time/src/tempo.rs`, `crates/model/src/session.rs` |
| UI | `crates/ui-egui/src/edit_window.rs`, `theme.rs`, `widgets.rs`, `fonts.rs`, `lib.rs`, `windows.rs` |
| Control and automation | `apps/soundcraft/src/control_server.rs`, `crates/automation/src/lib.rs`, `apps/soundcraft-web/src/web.rs` |

## Building so that merges stay small

- **AI features are additions.** New crates and modules (companion, contracts, analysis, policy, the
  assistant panel, vocal inspector, change review), reached through one or two small hooks in shared code.
- **Engine hooks are generic.** Session instance ID, content revision, guarded group apply: written so
  they could be offered upstream as ordinary pull requests, which removes them from what we carry.
- **UI: new screens in new modules.** Restyling an existing window edits a hot file and will conflict;
  prefer an opt-in theme over editing shared widgets, and offer it upstream.
- **Offer generic work upstream.** Audit fixes, the worker pool and similar changes are candidates. Each one
  accepted is one fewer thing to merge forever.

## Things only a person can decide

- Whether the fork follows upstream's decision to drop Pro Tools wording from its README and docs
  (`AGENTS.md` and the parity pages still use it).
- Whether to build the assistant on upstream's own (PR #603, "Craft AI") or beside it.
