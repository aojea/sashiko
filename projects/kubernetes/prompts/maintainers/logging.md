+++
title = "Logging and Error Reporting"
paths = [
    "staging/src/k8s.io/component-base/logs/",
]
symbols = [
    "HandleError",
    "HandleErrorWithContext",
    "HandleErrorWithLogger",
    "klog",
    "logger",
]
+++

# Logging and Error Reporting

Applies to code that logs or reports errors. In this tree the identifier
`logger` names a contextual logger, which is why it selects this note. The
`logcheck` linter enforces structured and contextual logging calls in the
packages listed in `hack/logcheck.conf`. It does not see the defects below.

## Log or return, not both

Source: kubernetes/community: contributors/devel/sig-instrumentation/logging.md
(When not to log?, What method to use?);
staging/src/k8s.io/apimachinery/pkg/util/runtime/runtime.go.

- A function that returns an error does not also log it as an error. The caller
  decides, and logging at both levels reports one failure twice.
- Library code returns errors and does not log them.
- Where an error cannot be returned, at the top of a goroutine or in a callback,
  it goes to `utilruntime.HandleErrorWithContext` or `HandleErrorWithLogger`.
  The error is passed as it is, not wrapped with `fmt.Errorf`.
- With a contextual logger, `logger.Error` is always written, whatever the
  verbosity. An error that is expected during normal operation is logged with
  `logger.V(n).Info(..., "err", err)`.

## Contextual logging pitfalls

Source: kubernetes/enhancements:
keps/sig-instrumentation/3077-contextual-logging/README.md (Pitfalls during
usage); kubernetes/community:
contributors/devel/sig-instrumentation/migration-to-structured-logging.md
(Contextual logging in Kubernetes).

The KEP states that code review must catch these patterns:

- A logger that got new values or a new name is not stored back into the
  context, so callees log without them.
- `logger = logger.WithValues(...)` inside a loop. Each iteration adds to the
  logger of the previous one. Use `:=` to declare a logger for the iteration.
- The result of `WithName` or `WithValues` is discarded.

Also check:

- A function that has a `ctx` gets its logger with `klog.FromContext(ctx)`. A
  new `context.TODO()` or `context.Background()` below a caller that has a
  context drops the logger and the cancellation.

## What is logged

Source: kubernetes/community: contributors/devel/sig-instrumentation/logging.md
(How to log, What method to use?); kubernetes/enhancements:
keps/sig-instrumentation/1753-logs-sanitization/README.md.

- Key and value arguments come in pairs, and keys are strings. An odd count
  loses a value.
- Objects are logged with `klog.KObj`, `klog.KObjSlice`, or `klog.KRef`, not with
  a string built by hand.
- No credential, token, or secret content is logged. No presubmit checks this.
  Flag a log call whose value is a Secret, a token, an authorization header, a
  kubeconfig, or a whole object that can contain one.
- The practical default level is `V(2)`, so a message at `V(2)` or below is
  written in every production cluster. Those levels are for changes of state.
  Flag a new message at `V(2)` or below that is written on every sync of every
  object when nothing changed. Debug output belongs at `V(4)` and above.
