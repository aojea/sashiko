+++
title = "Tests"
paths = [
    "**/*_test.go",
    "*_test.go",
    "test/",
]
+++

# Tests

Applies to unit, integration, and end-to-end tests. Kubernetes CI blocks merges
on failing tests, so a test that fails at random costs every contributor. Report
test findings at `Low`, unless the test passes when the code is wrong or leaks
state into other tests.

## Unit tests

Source: kubernetes/community: contributors/devel/sig-testing/testing.md (Unit
tests); kubernetes/community: contributors/guide/coding-conventions.md (Testing
conventions).

- A unit test is hermetic: it uses only what is in the test binary. No network,
  no cluster, no fixed port, no file outside a temporary directory.
- Unit tests run concurrently. Flag a test that writes a package variable, an
  environment variable, or a global registry without restoring it.
- Unit tests also run on macOS and Windows. A test of a Linux-only feature is
  skipped or compiled out on other platforms.
- Several scenarios are written as a table-driven test.
- A test does not assume that asynchronous work has finished. It waits and
  retries.
- A test for a bug fix fails without the fix. Flag a test that asserts nothing
  about the behavior that changed.

## Timing and flakes

Source: kubernetes/community: contributors/devel/sig-testing/flaky-tests.md
(Avoiding Flakes).

- Nothing happens instantly or after a fixed delay. Flag `time.Sleep` used to
  wait for a goroutine, a watch handler, or a sync loop. The test polls for the
  condition, with a timeout so that it does not wait forever.
- Map iteration has no fixed order. Flag an assertion that depends on the order
  of work done while iterating over a map.
- A test works when other tests run in parallel with it.

## Integration tests

Source: kubernetes/community: contributors/devel/sig-testing/integration-tests.md.

- Each integration test creates its own API server and configuration. Tests do
  not share state.

## End-to-end tests

Source: kubernetes/community:
contributors/devel/sig-testing/writing-good-e2e-tests.md (Debuggability, Polling
and timeouts, Speed of execution, Ability to run in non-dedicated test
clusters); kubernetes/community: contributors/devel/sig-testing/e2e-tests.md
(Kinds of tests).

- A failure explains itself. `framework.ExpectNoError(err, "...")` carries a
  message that says what was being done.
- Polling takes the context that Ginkgo passes to the test, so that an aborted
  test stops. Flag a new call to a poll function that takes no context.
  `gomega.Eventually` with the context is preferred.
- A test has no long sleeps. A test that is slow by design says why in a comment
  and is marked `[Slow]`.
- A test that changes the cluster for other tests, for example by restarting a
  component or changing a node, is marked `[Serial]` or `[Disruptive]`.
- A test does not depend on specific Events, their reasons, or their messages.
- `[Flaky]` is used rarely and comes with a reference to the issue for the fix.
- A test for a feature behind a gate is marked with `framework.WithFeatureGate`.
  The mark is removed when the gate is removed.

## Conformance tests

Source: kubernetes/community:
contributors/devel/sig-architecture/conformance-tests.md (Conformance Test
Requirements).

- A conformance test covers only GA features and APIs that are not optional. It
  needs no feature flag and uses no deprecated feature.
- It works for every provider and has no `SkipIfProviderIs` or
  `SkipUnlessProviderIs` call.
- It does not depend on the kubelet API and does not check for specific Events.
- Flag a change that promotes a test to conformance and breaks one of these.
