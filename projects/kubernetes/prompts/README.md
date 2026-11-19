# Kubernetes Review Prompts

First-party review prompts for reviewing commits to
[kubernetes/kubernetes](https://github.com/kubernetes/kubernetes), used by the
`kubernetes` project profile (`--project kubernetes`). The review runs locally
against a Kubernetes checkout; there is no hosted Kubernetes instance.

## Layout (three layers)

| Path | Loaded by |
|---|---|
| `review-core.md` | Orientation: what Kubernetes is and how to review it (system prompt, every stage) |
| `false-positive-guide.md`, `severity.md` | The verification and post-verification stages |
| `report-template.md` | The report stage |
| `maintainers/*.md` | Maintainer notes: human-curated policy, scoped by `+++` front matter (`paths`, `symbols`) and selected deterministically from the patch diff |
| `notes/` | Generated code notes (source-verified codebase facts); not yet generated for Kubernetes |

Maintainer notes are layer 2 of the three-layer prompt architecture: they
state what maintainers require, forbid, or deliberately suppress, never
volatile implementation facts, which belong in generated code notes. Notes
outside `general.md` load through `paths` prefixes or specific `symbols`. Some
prefixes are wide on purpose: `compatibility` covers the code of every
component, since its rules apply to any code there.

A symbol trigger is one Go identifier. The diff tokenizer splits a qualified
name at the dot, so `cache.WaitForCacheSync` is matched by `WaitForCacheSync`.
Do not add a trigger that most Go diffs contain, such as `go` (every import of
`k8s.io/client-go` has it), `ctx`, or `err`. Do not add a method name that
unrelated APIs share: `WithName` is a logger method and also a setter in every
generated apply configuration.

## Sources

Every section names the document it comes from, so a reviewer can check it.
Kubernetes keeps much of its contributor guidance outside the tree, so a source
is one of:

- a path relative to the root of the Kubernetes tree, such as
  `pkg/features/kube_features.go`;
- `kubernetes/community:` followed by a path in that repository;
- `kubernetes/website:` followed by a path in that repository, for pages
  published on kubernetes.io;
- `kubernetes/contributor-site:` followed by a path in that repository, for
  pages published on kubernetes.dev;
- `kubernetes/enhancements:` followed by the path of a KEP;
- `kubernetes/committee-security-response:` followed by a path in that
  repository.

When a document changes, update the rule to match. The tree under review is
always authoritative over these notes: several of the documents name
identifiers that the tree has since renamed.

## What is not checked

- **Pull request metadata.** Local review reads commits. The release note
  block, the `/kind` label, and the AI usage disclosure live in the pull request
  description and are out of reach.
- **Sign-off.** Kubernetes requires a signed CLA, not a `Signed-off-by` trailer.
  The trailer is neither required nor reported.
- **What `make verify` reports.** See `false-positive-guide.md`.

## Editing

Prompt content is embedded in the binary at build time and installed to
`$XDG_DATA_HOME/sashiko/prompts/<revision>/kubernetes/`. After editing a file
here, rebuild and run the new binary. If using the installed `sashiko` command,
repeat the installation command in
[the setup guide](../../../docs/kubernetes-review.md#setup). The new binary
installs its updated bundle into a directory named by the new content
fingerprint; an existing binary continues to use its embedded version.

For prompt iteration without rebuilding, pass `--prompts` with the absolute path
to this directory when running a review from the Kubernetes checkout.

## Writing a new maintainer note

Add the file under `maintainers/` with `+++` TOML front matter (`title`, plus
`paths` and/or `symbols` triggers), one rule per bullet. A rule belongs here if
it is verifiably true of Kubernetes, names real identifiers or files, cites
where it comes from, and helps decide whether a specific change is wrong.
Generic Go style advice does not belong here, and neither does anything
`make verify` already enforces.

Notes for one SIG's area, such as the scheduler, networking, or storage, are
welcome. They are best written by the people who review that code.
