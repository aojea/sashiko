+++
title = "Kubernetes Repository-Wide Maintainer Conventions"
paths = [
    "*",
]
+++

# Kubernetes Repository-Wide Conventions

Cross-cutting rules that apply to every Kubernetes change.

## Commit messages

Source: kubernetes/community: contributors/guide/pull-requests.md (Commit
Message Guidelines); kubernetes/community: contributors/guide/contributing.md
(Best practices); AGENTS.md (Contributor Guidelines).

- The subject uses the imperative mood and does not end with a period. Its first
  word is capitalized, unless the subject starts with an identifier or an area
  such as `kubelet:`. Aim for 50 characters or fewer. Flag a subject longer than
  72 characters.
- An area prefix is common in the history and is not required. Do not flag a
  subject for lacking one.
- A blank line separates the subject from the body, and the body is wrapped at 72
  characters.
- The message explains what changed and why. A subject alone is enough only when
  it says both in full, which is usual for a trivial change. Flag a message for
  a non-trivial change that restates the diff or gives no reason.
- A commit message has no GitHub keyword (`close`, `fix`, `resolve`, and their
  other forms) followed by an issue number, no `#` reference to a pull request,
  and no `@` mention. These belong in the pull request description. A plain link
  to an issue in the body is fine.
- Report commit message findings at `Low`.

## Provenance

Source: kubernetes/community: CLA.md; kubernetes/community:
contributors/guide/pull-requests.md (AI Guidance).

- Kubernetes requires a signed Contributor License Agreement. No document
  requires a commit trailer. Do not report a missing `Signed-off-by` trailer, and
  do not report one that is present.
- A commit must not list an AI tool as a co-author or co-signer, and must not
  carry an `Assisted-by:`, `Co-developed-by:`, or similar trailer for one. Flag
  such a trailer at `Low`. A `Co-authored-by:` trailer that names a person is
  normal.
- AI-generated commit messages are not allowed, but the text alone does not show
  how a message was written. Do not guess from its style.

## Commit structure

Source: kubernetes/community: contributors/guide/pull-requests.md (Smaller Is
Better, Squashing); AGENTS.md (Contributor Guidelines).

- Changes that are unrelated to the purpose of the commit belong in a different
  pull request. Flag a commit that bundles unrelated changes.
- Commits that fix earlier commits of the same series are squashed into them.
  Independent changes layered toward one goal stay separate. In a range, flag a
  commit whose only purpose is to correct an earlier commit of the range.

## Tests with code

Source: kubernetes/community: contributors/guide/pull-requests.md (Test);
AGENTS.md (Contributor Guidelines).

- A change to code normally changes tests too. For a bug fix, the test fails
  without the fix. Flag a behavior change with no test in the commit or in a
  later commit of the series, at `Low`.
- Do not ask for a trivial unit test whose only purpose is coverage.
