+++
title = "Metrics"
paths = [
    "**/metrics.go",
    "hack/tools/instrumentation/",
    "staging/src/k8s.io/component-base/metrics/",
]
symbols = [
    "CounterOpts",
    "DeprecatedVersion",
    "GaugeOpts",
    "HistogramOpts",
    "NewCounter",
    "NewCounterVec",
    "NewDesc",
    "NewGauge",
    "NewGaugeVec",
    "NewHistogram",
    "NewHistogramVec",
    "StabilityLevel",
    "SummaryOpts",
    "WithLabelValues",
    "legacyregistry",
]
+++

# Metrics

Applies to a change that adds, changes, or removes a metric. Metric names and
changes to stable metrics are checked by `hack/verify-metrics-naming.sh` and
`hack/verify-generated-stable-metrics.sh`. Label values are not checked by
anything.

## Cardinality

Source: kubernetes/community:
contributors/devel/sig-instrumentation/metric-instrumentation.md (Dimensionality
& Cardinality, Normalization).

- Every label has a set of values that is known and bounded when the metric is
  written. Trace each value passed to `WithLabelValues` back to its origin.
- A label value is never a user ID, an error message, or the name of a pod, a
  node, or a namespace in a component metric. Each distinct value is a new time
  series that the component holds in memory until it restarts.
- Flag a label value that comes from an object field or a request that users
  control, such as a resource name, a path, or a user agent. Name the field and
  who sets it.
- One label holds one value. Do not join several values into one label.

## Stability

Source: kubernetes/community:
contributors/devel/sig-instrumentation/metric-stability.md (Beta, Stable, Metric
Renaming, Deprecation Lifecycle).

- No label is removed from a beta metric.
- No label is added to or removed from a stable metric, and its type does not
  change.
- A rename is a removal and an addition. It is not allowed for a stable metric.
- A metric is deprecated before it is removed: it is marked with
  `DeprecatedVersion`, then hidden, then deleted. A stable metric stays for 3
  releases or 9 months after the deprecation, a beta metric for 1 release or 4
  months.
- A deprecation names the replacement in the help text.
- Flag a change that deletes or renames a metric, or changes its labels, without
  following these steps. Dashboards and alerts read these names.

## Tests

Source: kubernetes/community:
contributors/devel/sig-instrumentation/metric-instrumentation.md (Adding a New
Metric).

- A metric test checks the values the code produces, for example with
  `testutil.GatherAndCompare`. A test that only checks that the metric is
  registered proves nothing.
