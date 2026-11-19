+++
title = "Dependencies and Vendored Code"
paths = [
    "**/go.mod",
    "**/go.sum",
    "LICENSES/",
    "go.mod",
    "go.sum",
    "go.work",
    "go.work.sum",
    "hack/unwanted-dependencies.json",
    "vendor/",
]
+++

# Dependencies and Vendored Code

Applies to a change of a module requirement or of vendored code.

Source: kubernetes/community: contributors/devel/sig-architecture/vendor.md
(Justifications for an update); AGENTS.md (Constraints); kubernetes/community:
contributors/devel/sig-release/cherry-picks.md (What Kind of PRs are Good for
Cherry Picks).

- `go.mod`, `go.sum`, `go.work`, and `vendor/` are written by
  `hack/pin-dependency.sh` and `hack/update-vendor.sh`. `make verify` reports a
  tree where they disagree. Do not review vendored source line by line, and do
  not report defects in it as defects of this change.
- Review the decision. The project accepts a new or updated dependency for a
  reason: functionality that is needed, a bug or security fix, a performance
  gain, a requirement of another dependency, or compatibility with other
  projects. Flag a dependency change whose commit message gives no reason.
- A new dependency is code that every cluster runs. Check what the change makes
  it do: whether it runs in a control plane component, and whether it handles
  input that users control.
- A change to `hack/unwanted-dependencies.json` that allows a module the project
  had marked as unwanted needs its reason in the commit message.
- On a release branch, a dependency update that only quiets a scanner and does
  not fix code that is vulnerable is not eligible.
