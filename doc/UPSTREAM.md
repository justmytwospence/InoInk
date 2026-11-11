# Upstream provenance

InoInk starts from Plato 0.9.45, `master` revision
`7da89f296e58c7426692abb6fda16431b9631e60` (https://github.com/baskerville/plato),
with its full history preserved. Plato is licensed AGPL-3.0-or-later (`LICENSE-AGPLv3`);
InoInk keeps that license and Plato's copyright notices.

## Policy

InoInk is a derivative, not a rebased patch stack. Product commits do not need to apply to
upstream, and there is no routine rebase. Renderer, typography, display/input, frontlight and
power code stays recognizable so relevant upstream fixes can be applied selectively:

```sh
git fetch upstream
git log --oneline 7da89f2..upstream/master -- crates/core/src/document crates/core/src/framebuffer \
  crates/core/src/input.rs crates/core/src/frontlight crates/core/src/device.rs
git cherry-pick -x <sha>      # review each fix; record it below
```

## Selectively applied upstream changes

None yet.
