# Design: Kubernetes Review

## 1. Overview

Add Kubernetes as a Sashiko project, so that a contributor can run
`sashiko review --project kubernetes <commit-or-range>` in a checkout of
`kubernetes/kubernetes`. This is initial support for local review. It follows
the per-project structure in `DESIGN_MULTI_PROJECT_REVIEW.md` and the shape of
`DESIGN_ZEPHYR_REVIEW.md`.

Kubernetes differs from the projects Sashiko reviews today in four ways that
shape this design: it is written in Go, it is a distributed control plane whose
components run at different versions, its API is a compatibility contract with
every cluster that stores objects, and its contribution process uses a CLA and
GitHub pull requests instead of DCO sign-off and mail.

### 1.1 Goals

1. `ProjectId::Kubernetes`, selected like any other project.
2. A Kubernetes review workflow with stages aimed at the defects of a Go
   control plane: API compatibility, reconciliation, concurrency, resource
   lifetime, security boundaries, and tests.
3. A `projects/kubernetes/prompts/` tree in which each rule cites the Kubernetes
   document or source file it comes from.
4. No `Signed-off-by` audit. Kubernetes contributors sign a CLA.

### 1.2 Non-goals

- A hosted instance. Kubernetes uses GitHub pull requests and Prow; ingesting
  them is daemon work with open gaps (section 7).
  `projects/kubernetes/mailing_lists.toml` exists because every project has
  one, and it tracks nothing.
- Daemon review. Baseline resolution knows the Kubernetes remote and its
  `master` branch, because the code paths are shared. A daemon was run against
  `kubernetes/kubernetes` only with `--no-ai`, to check that pull requests are
  polled, filtered, and fetched. No review was run in a daemon.
- Other repositories of the `kubernetes` and `kubernetes-sigs` organizations.
  The staging repositories are in scope because their source of truth is
  `staging/src/k8s.io/` in this tree.
- Removal of generated and vendored files from the diff (section 5).
- A benchmark suite and generated code notes (section 7).

## 2. Workflow

`src/workflows/kubernetes_patch_review.rs` is a sibling of
`zephyr_patch_review.rs`: same graph, same state type, same consolidation
pipeline. Forking matches the decision in `DESIGN_MULTI_PROJECT_REVIEW.md`. It
imports the project-neutral concern validators and plain-text report helpers
from `qemu_patch_review.rs`, as the Zephyr workflow does.

The Sashiko workflow is the other GitHub-hosted sibling, with deduplication,
verification shards, prior-version attribution, and a Markdown summary. Those
stages serve a daemon that posts on pull requests. Local review does not need
them, so the smaller graph is the base. A hosted mode can add them later.

### 2.1 Stages

| Stage | Optional | Scope |
|---|---|---|
| `goal` | no | Intent, fit with Kubernetes patterns, KEP and feature gate expectations, commit message |
| `implementation` | no | Does the code do what the message claims; touch points of a new field, gate, flag, or metric |
| `execution-flow` | no | Control flow, nil handling, error handling, Go pitfalls |
| `api-compat` | yes | API types, defaulting, validation, conversion, feature gates, version skew, upgrade and rollback |
| `reconciliation` | yes | Control loops: level-triggered logic, informer caches, workqueues, status updates, API call volume |
| `concurrency` | yes | Goroutines, locks, channels, context cancellation, shared cache objects |
| `resources` | yes | Goroutine, watch, timer, and file leaks; unbounded growth; cleanup of node and API objects |
| `security` | yes | Authentication, authorization, admission, the node trust boundary, secrets, untrusted object fields |
| `tests` | yes | Whether a test can detect the defect, flakiness, feature gate setup, test level |

`api-compat` has no Zephyr counterpart. A Kubernetes API change can build,
pass unit tests, and still break clusters: a stricter validation rule rejects
updates to stored objects, a field written by a new API server is dropped by an
old one, a kubelet three minor versions behind does not know a new field. The
stage asks for series context because generated code and gate wiring often
arrive in a later commit of the same pull request.

`reconciliation` is separate from `concurrency` because its defects are not
data races. A controller that reacts to an event instead of the current state,
that trusts a stale cache read, that requeues without backoff, or that lists
from the API server per object is wrong with one goroutine.

`resources` covers leaks in a garbage-collected language: goroutines, watches,
timers, HTTP response bodies, file descriptors, and maps that grow with cluster
size. It also covers the external state that components own, such as mounts,
cgroups, network rules, and finalizers that block deletion.

`tests` is a stage because flaky and non-detecting tests are a first-order cost
for Kubernetes: CI blocks merges on them.

### 2.2 Differences from the Zephyr prompts

- **No sign-off audit.** The goal, verification, and post-verification stages
  carry no `Signed-off-by` rule, and `fixes_check` renders no sign-off section
  (section 4).
- **CI noise.** The consolidation stages discard what `make verify` reports:
  the `hack/verify-*.sh` scripts, `golangci-lint`, type checking, and stale
  generated files. A logic error that passes them is not noise.
- **Finding titles.** A title starts with the component or area prefix used in
  the history of the affected files, such as `kubelet:`, `scheduler:`,
  `apiserver:`, `client-go:`, `kubectl:`, or `api:`. About half of recent
  commits use such a prefix, so the goal stage does not require one in the
  commit subject.

## 3. Prompts

`maintainers/` holds one note per topic, selected from the diff:

| Note | Loaded by |
|---|---|
| `general` | every patch |
| `api-changes` | API type packages under `staging/src/k8s.io/api/` and `pkg/apis/`, and sibling API groups in staging |
| `api-validation` | validation, defaulting, and registry strategy code |
| `feature-gates` | `pkg/features/`, feature gate packages in staging, or feature gate symbols |
| `compatibility` | component code under `cmd/` and `pkg/`, and the staging repositories of the components |
| `controllers` | `pkg/controller/`, client-go caches and workqueues, or informer and workqueue symbols |
| `logging` | `klog` and contextual logging symbols |
| `metrics` | metrics packages, or metric constructor symbols |
| `kubelet` | `pkg/kubelet/`, `cmd/kubelet/`, the CRI API |
| `security` | authentication, authorization, admission, RBAC bootstrap policy, service account and certificate code |
| `dependencies` | `go.mod`, `go.sum`, `go.work`, `vendor/` |
| `tests` | `test/`, `*_test.go` |

Symbol triggers are single Go identifiers, because the diff tokenizer splits a
qualified name such as `cache.WaitForCacheSync` at the dot. Triggers that would
match most Go diffs, such as `go` (part of `client-go` in every import block),
`ctx`, and `err`, are not used.

Each section names its source. Kubernetes keeps much of its contributor
guidance outside the tree, so a source is one of:

- a path in the tree under review, such as `pkg/features/kube_features.go`;
- a repository of the `kubernetes` organization followed by a path in it:
  `kubernetes/community:`, `kubernetes/website:`, `kubernetes/contributor-site:`,
  `kubernetes/enhancements:`, or `kubernetes/committee-security-response:`.

The rules were collected from those documents and each quoted sentence was
checked against the fetched text. Every path and Go identifier that a prompt
names was then checked against `kubernetes/kubernetes` at `master`. A rule that
could not be traced to a source was left out. Two examples of what that
removed: no document promises a stable Go API for the staging libraries, so
there is no note that asks for one, and no document states a rule for fake
clocks in tests. The tree under review is authoritative over a note, and the
README says so: some of the documents name identifiers that the tree has since
renamed.

The tree has its own `AGENTS.md`. Its constraints on generated files,
dependencies, and commit messages are cited where they apply.

Per-SIG notes for the scheduler, networking, storage, and API machinery
internals are not in the first set. They need rules from the maintainers of
those areas, not from a general reading of the documents.

## 4. Commit and provenance policy

Kubernetes requires a signed CLA and does not require a `Signed-off-by`
trailer. Roughly a quarter of recent commits carry one by habit, so the trailer
is neither required nor reported.

`analyze_fixes_for_commit()` selects projects that get trailer checks by an
explicit list. Kubernetes is not on it, so it takes the same path as GCC: series
and author evidence is collected for the sanitizer, and no sign-off or `Fixes:`
section reaches the prompt. A test pins this, so that a later edit of the list
cannot turn the audit on by accident. Kubernetes links fixes to issues in the
pull request description, not with a `Fixes: <sha>` trailer, so proactive
trailer checks would have nothing to check.

The goal stage audits what the Kubernetes documents do ask of a commit message:
an imperative subject of at most 72 characters, a body that says what and why,
no GitHub keywords or mentions, and no trailer that lists an AI tool. These
findings are `Low`.

Local review reads commits, not pull requests. Rules that apply to the pull
request description, such as the release note block and the AI disclosure, are
out of reach and are not checked.

### 4.1 Severity

`severity.md` takes two calibration rules from Kubernetes documents. The
Security Response Committee rates an attack lower when it needs more access, so
a finding names the identity that can trigger it. The cherry-pick guide does not
backport a fix for a defect that exists only while an off-by-default alpha gate
is enabled, so such a finding is capped at `Medium`, unless it changes behavior
with the gate off or leaves stored data behind.

## 5. Generated and vendored files

The tree lists its generated files in `.generated_files`: `zz_generated.*`,
`generated.pb.go`, `generated.proto`, `types_swagger_doc_generated.go`,
`pkg/generated/`, and `vendor/`. `.gitattributes` adds `api/openapi-spec/`.

A commit that changes an API type often carries more generated lines than
written ones. This design leaves them in the diff and tells the reviewer how to
treat them:

- Generated files are not a review subject. `hack/verify-codegen.sh` and its
  siblings report stale output, so a suspected mismatch is CI noise.
- They are evidence. A regenerated conversion or defaulting function shows what
  a hand-written change to `types.go` does.
- `vendor/` is reviewed only as a dependency change, through `go.mod`.

Removing these files from the prompt needs a per-file diff filter, which does
not exist: `review.ignore_files` skips a patch only when every file matches.
That filter is the first follow-up, and `.generated_files` is its natural input.

## 6. Touch points

Adding the variant forces an arm in every `match` on `ProjectId`, including the
bug pipeline in `linux_bug.rs`, which local review with `--report-preexisting`
does not reach but which must still compile. Those arms carry Kubernetes wording
rather than a shared fallback, so a future daemon does not inherit another
project's vocabulary.

Three places are not checked by the compiler:

- `db.rs` keeps audit tools in SQL `CASE` expressions. Each Zephyr clause gets a
  Kubernetes sibling, and a test covers them.
- `target_project_for_bug()` and `infer_project_from_bug_or_tool()` map a
  `kubernetes-` bug identifier and a `sashiko:kubernetes` tool to the project.
- `is_non_linux_project_url()` in `baseline.rs` keeps a Kubernetes remote from
  being treated as a Linux tree.

Subsystem labels come from the first two path components. That gives every
staging repository the label `staging/src`, so paths under
`staging/src/k8s.io/<repo>/` use four components.

## 7. Open questions

- **Hosted review.** The forge code keeps a pull request title and its labels
  but not its description or draft state, and it does not know the Prow bots.
  Automatic review can be limited to listed authors and labels
  (`DESIGN_FORGE_REVIEW_FILTERS.md`), which is enough for a private instance.
  A filter on changed paths does not exist. All of these matter for
  `kubernetes/kubernetes`.
- **Generated files.** See section 5.
- **Benchmark.** Kubernetes has no `Fixes: <sha>` trailers. Reverts, cherry-pick
  pull requests, and the `kind/regression` label are candidate sources for pairs
  of an introducing commit and its fix.
- **Code notes.** Not generated yet. The notes engine extracts entry points for
  Rust, C, and C++ only, so Go needs support there first.
- **Scale.** A dedicated performance stage, as Linux has, may be worth its cost
  for API server and scheduler hot paths. `reconciliation` and `resources`
  cover the common cases until there is evidence.
