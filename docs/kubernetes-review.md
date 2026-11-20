# Reviewing Kubernetes

Sashiko can review commits in a local checkout of
[Kubernetes](https://github.com/kubernetes/kubernetes). Support is a pilot
covering local review only: there is no public hosted Kubernetes instance, and
Sashiko does not ingest Kubernetes pull requests.

> [!IMPORTANT]
> Review sends the commit and the surrounding source to your LLM provider. In
> testing with Gemini, a 30-line fix took 0.6 million input tokens and three
> and a half minutes. A 440-line feature commit with ten findings took 6.4
> million input tokens. Start with a small commit to see what a review costs on
> your plan.

## Setup

Install Sashiko from a checkout. Local review needs none of the daemon features, and
the `cache` feature is what makes `--resume` work:

```bash
cargo install --path . --locked --no-default-features --features cache --bin sashiko
```

Get the Kubernetes source, if you do not have it. The commits you review must be
in this repository's history:

```bash
git clone https://github.com/kubernetes/kubernetes.git
```

A fork works too. Sashiko finds the upstream tree by the URL of a remote, so
add one that points at `kubernetes/kubernetes` and fetch it:

```bash
git remote add upstream https://github.com/kubernetes/kubernetes.git
git fetch upstream master
```

Configure a provider.
[`docs/examples/Settings.kubernetes.toml`](examples/Settings.kubernetes.toml)
uses the Gemini API, which is what this support was tested with; see
[LLM Provider Configuration](llm-providers.md) for the others. Copy it to a
separate file and pass it with `--settings`. The command below leaves an
existing Kubernetes settings file unchanged:

```bash
mkdir -p ~/.config
cp -n docs/examples/Settings.kubernetes.toml ~/.config/sashiko-kubernetes.toml
export GEMINI_API_KEY="your-gemini-api-key"
```

Setting `kind = "kubernetes"` under `[project]` makes `--project kubernetes` the
default for that file, and it makes a run with a different `--project` fail.
Without it, pass `--project kubernetes` each time, or set
`SASHIKO_PROJECT=kubernetes`.

## Reviewing commits

Run the review from inside the Kubernetes checkout:

```bash
# Review the latest commit
sashiko review --project kubernetes --settings ~/.config/sashiko-kubernetes.toml

# Review one commit
sashiko review --project kubernetes --settings ~/.config/sashiko-kubernetes.toml 6f89451b0c3

# Review the commits of a branch; each commit is checked separately
sashiko review --project kubernetes --settings ~/.config/sashiko-kubernetes.toml upstream/master..HEAD

# Combine analysis domains into one pass; keep verification stages
sashiko review --project kubernetes --settings ~/.config/sashiko-kubernetes.toml --cheap HEAD

# Also report bugs that existed before the change
sashiko review --project kubernetes --settings ~/.config/sashiko-kubernetes.toml --report-preexisting HEAD

# Continue a review cut short by a quota stop, a timeout or Ctrl-C
sashiko review --project kubernetes --settings ~/.config/sashiko-kubernetes.toml upstream/master..HEAD --resume
```

Review reads commits from git. It does not build Kubernetes, does not apply
patches to your working tree, and does not change it. Lower
`review.concurrency` if you hit rate limits.

Review a range when a pull request has several commits. A later commit often
carries the generated code, the feature gate wiring, or the tests of an earlier
one, and the review of each commit is told which commits follow it.

## Reading the results

Progress goes to stderr and the report to stdout. For structured findings, use
`--format json`. A human-readable report may include bullets such as:

```text
- [HIGH] In resourceslicecontroller.go
  (staging/src/k8s.io/dynamic-resource-allocation/resourceslice,
  filterSliceWatchByPoolName), the watch predicate drops any event whose
  object is not a ResourceSlice with Spec.Pool.Name equal to poolName. When
  ReconcilePoolWithName is set, this filters out watch.Bookmark events,
  which prevents WatchList initial cache sync from completing, and drops
  watch.Error events, preventing the reflector from handling server watch
  errors such as 410 Gone. Pass through events where event.Type is
  watch.Bookmark or watch.Error before filtering by pool name.

- [MEDIUM] In resourceslicecontroller.go (Controller.Update),
  c.errorHandler is invoked while holding c.mutex.Lock(). An ErrorHandler
  that calls Update synchronously deadlocks on the non-reentrant mutex.
  Release the lock before invoking the callback.
```

Both bullets are shortened from a review of Kubernetes commit `6f89451b0c3`.
Kubernetes fixed the first defect in `3d5aa1c7702` and the second in
`6e13e157ccb`.

`Critical` is reserved for an identity that gets past a trust boundary, and for
stored cluster state that is lost beyond repair. `High` is a crash, a deadlock,
data loss, workload disruption, or a compatibility break on a realistic path. A
defect that exists only while an off-by-default alpha feature gate is enabled is
capped at `Medium`. Findings marked `(pre-existing)` were not introduced by the
change.

The command exits with 1 if a new `High` or `Critical` finding is reported.
Pre-existing findings do not change the exit status. Exit 3 means the review did
not complete, for example at a usage limit.

Treat a report as a list of candidates and check each one against the code. The
review cannot see container runtimes, CNI and CSI plugins, cloud providers, or
etcd, and it does not run CI, so `make verify` and the tests are still needed.

## What the review covers

By default, three analysis stages always run: `goal`, `implementation`, and
`execution-flow`. A planning stage adds these when the patch calls for them:

| Stage | Looks for |
|---|---|
| `api-compat` | Stored objects that stop validating or converting, defaults that change meaning, fields of a disabled feature gate that are not cleared, version skew between components, upgrade and rollback |
| `reconciliation` | Control loops that depend on events instead of state, stale cache reads, cache objects changed in place, workqueue misuse, finalizers and ownership, API calls per object |
| `concurrency` | Data races, locks held across blocking calls, goroutines with nothing to stop them, lost context cancellation |
| `resources` | Leaked goroutines, watches, timers and files, unbounded maps and metric labels, node and cloud state that is not cleaned up |
| `security` | Authorization and admission checks that can be skipped, the node boundary, fields that users write and privileged components use, secrets in logs |
| `tests` | Tests that cannot fail, fixed sleeps, shared state between tests, feature gate setup |

With `--cheap`, one combined analysis pass covers all nine domains and skips
planning. Verification and post-verification still run for the resulting findings.

Findings are checked against the policy in
[`projects/kubernetes/prompts/`](../projects/kubernetes/prompts/README.md). Each
maintainer note there names the Kubernetes document or source file it comes
from, and is loaded only for patches it applies to.

## What the review does not do

- **No `Signed-off-by` check.** Kubernetes requires a signed CLA and no commit
  trailer. The commit message is checked against the Kubernetes commit message
  guidelines instead, and those findings are `Low`.
- **No pull request metadata.** The release note, the `/kind` label, and the AI
  usage disclosure are in the pull request description, which a local review
  does not read.
- **Generated and vendored files are not a review subject.** The files named in
  `.generated_files`, `api/openapi-spec/`, and `vendor/` stay in the diff and
  are read only as evidence of what a hand-written change does. They still cost
  tokens: a commit that only regenerates code is cheaper to skip.
- **Nothing that `make verify` reports.** Formatting, lint, type errors, stale
  generated code, and feature gate list entries are left to CI.

Not covered yet: generated code notes, a benchmark suite, notes for one SIG's
area such as the scheduler, networking, or storage, and other repositories of
the Kubernetes organizations.
