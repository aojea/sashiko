# False Positive Prevention Guide

This guide is used where avoiding false positives matters most. Shift bias away
from fast processing and follow it carefully.

## Core principle

**If you cannot prove an issue exists with concrete evidence from this
codebase, do not report it.**

Evidence means code you have read. Not "controllers usually", not "this looks
like it could", not "a caller might". Name the function, name the goroutine or
handler it runs in, name the concrete execution path that reaches it.

The corollary matters as much: proving a path is *structurally possible* is
enough. You do not have to prove it happens in every cluster. A user-written
field used as a path segment without validation is a bug even if no known user
writes `..` into it yet.

## Patterns that produce false positives here

### 1. Speculation without a scenario

If you cannot construct a specific request, object, or sequence of events that
triggers the issue, demote it to `Low` or drop it entirely. "This value is not
validated" is not a finding until you name who controls the value and what it
then breaks.

### 2. Restating obviously correct code

Do not report that code does what it says. A finding states a defect.

### 3. Issues CI already catches

`make verify` runs on every pull request. Do not report what it reports, unless
you have proved that the check would miss it:

- Formatting, license headers, spelling, import aliases, and the lint rules in
  `hack/golangci.yaml`, which include `logcheck` for structured and contextual
  logging in the packages listed in `hack/logcheck.conf`.
- Suspected compile errors and type errors on any platform
  (`hack/verify-typecheck.sh`), and unit tests that would not build.
- Stale generated code, OpenAPI specifications, and API fixtures
  (`hack/verify-codegen.sh`, `hack/verify-openapi-spec.sh`).
- Feature gate declarations: ordering, an alpha gate that is on by default, a
  gate removed too early, and a stale
  `test/compatibility_lifecycle/reference/versioned_feature_list.yaml`
  (`hack/verify-featuregates.sh`).
- Import direction and forbidden dependencies, including an import of
  `k8s.io/kubernetes` from `staging/` (`hack/verify-import-boss.sh`,
  `hack/verify-imports.sh`, `hack/verify-vendor.sh`,
  `hack/verify-publishing-bot.sh`).
- Metric naming and changes to stable metrics (`hack/verify-metrics-naming.sh`,
  `hack/verify-generated-stable-metrics.sh`).
- A `fixes #123` keyword in a commit message. Prow labels the pull request
  `do-not-merge/invalid-commit-message`. Report it at `Low`, because a local
  review runs before Prow does.

These checks do not see logic. A missed `cancel()` call, a use of a deprecated
function, a cache object mutated in place, a tombstone that is not handled, and
a requeue without backoff all pass `make verify`.

### 4. Generated and vendored files

Files named in `.generated_files`, `api/openapi-spec/`, and `vendor/` are tool
output. Do not report a defect in their content, a style issue in them, or a
mismatch with their source. If the generated code is wrong, the defect is in the
hand-written type, tag, or generator, and belongs there.

### 5. Code behind a feature gate you did not check

Before reporting that new behavior breaks existing clusters, find the gate that
guards it and read its default at the target commit. Behavior behind a gate
that is off by default does not change a cluster that leaves the gate alone.
Report what the gate does not contain: a path that runs with the gate off, data
that stays after the gate is turned off, or a missing gate check.

### 6. Reads from an informer cache

Reading from a lister is how controllers work, and the cache is allowed to lag.
A stale read is a finding only when it leads to a wrong action that the next
sync does not correct, such as deleting an object that was just adopted, or
creating a duplicate that nothing removes. Do not ask for a direct API read
where a later resync or a conflict error repairs the result.

### 7. Races that the queue or the worker model rules out

Before reporting a race, confirm that both sides can run at the same time. A
workqueue does not hand one key to two workers at once. The kubelet runs one
worker per pod. Many sync loops run in a single goroutine. A race needs two
goroutines that touch the same data without a lock between them: name both.

### 8. Fields that the API server guarantees

An object read from the API server passed validation and defaulting. Do not ask
for a nil check on a field that defaulting always sets, or for a range check
that validation already makes. This does not hold for a field or a default that
the patch itself adds: an older API server in the same control plane serves the
object without it. It also does not hold for objects built by tests, by static
pod files, or by the component itself.

### 9. Errors that are handled by convention

A sync function that returns its error is requeued with backoff by its caller.
`utilruntime.HandleError` and its variants report an error where none can be
returned; that is handling, not swallowing. Read the caller before reporting a
dropped error.

### 10. Go semantics

Check the `go` directive in `go.mod` before reporting a language pitfall. Each
iteration of a `for` loop has its own variable, so a closure or goroutine that
captures it is not a bug. Reading from a nil map, calling `len` or `range` on a
nil slice, and `append` to a nil slice are all valid. Writing to a nil map
panics. An interface holding a nil pointer is not equal to `nil`.

### 11. Version skew without version numbers

A skew finding must name the versions and the direction. Components are never
newer than the API server they talk to, so "an old API server with a new
kubelet" is not a supported state. A kubelet or kube-proxy can be three minor
versions older than the API server; the controller manager and the scheduler,
one.

### 12. Behavior outside the tree

Container runtimes, CNI and CSI plugins, cloud providers, and etcd are not in
this repository. Do not report that one of them misbehaves without evidence from
the tree, such as the CRI API comments, a fake implementation, or an existing
workaround. Otherwise mark the finding speculative.

### 13. Tests held to production standards

Test code may abbreviate production boilerplate. Report an omission when
concrete evidence shows a false pass, flakiness, a leak between tests, or
incorrect API use. Short test helpers alone are not defects.

### 14. A policy the commit states

A commit can change a default or a behavior on purpose. When the message states
the new policy, the behavior that follows from it is not a regression, even if
it differs from the parent. Report it only if the code does not do what the
message says, or if you can show the policy breaks something the message does
not account for, such as stored objects or an older component. A stated policy
never excuses a security, compatibility, or data loss defect.

### 15. Deprecated functions and style preferences

A call to a deprecated function is not a defect by itself, and existing code is
full of them. Report it at `Low`, and only for a call the patch adds when the
replacement fixes a concrete problem, such as a poll that ignores context
cancellation. Do not report a naming or structure preference that the
Kubernetes documents do not state.
