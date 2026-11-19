+++
title = "Feature Gates"
paths = [
    "pkg/features/",
    "staging/src/k8s.io/apiextensions-apiserver/pkg/features/",
    "staging/src/k8s.io/apiserver/pkg/features/",
    "staging/src/k8s.io/client-go/features/",
    "staging/src/k8s.io/component-base/featuregate/",
    "staging/src/k8s.io/controller-manager/pkg/features/",
    "test/compatibility_lifecycle/",
]
symbols = [
    "DefaultFeatureGate",
    "DefaultMutableFeatureGate",
    "SetFeatureGateDuringTest",
    "SetFeatureGateEmulationVersionDuringTest",
    "SetFeatureGateVersionsDuringTest",
    "SetFeatureGatesDuringTest",
    "featuregate",
    "featuregatetesting",
    "utilfeature",
]
+++

# Feature Gates

Applies to a change that declares a feature gate, changes its stage, or puts
behavior behind one. `hack/verify-featuregates.sh` checks the declarations:
ordering, an alpha gate that is on by default, and a gate removed too early.
Review the code paths instead.

## Lifecycle

Source: kubernetes/community: contributors/devel/sig-architecture/feature-gates.md
(Lifecycle, Compatibility versions, Features which add a new API field, GA
features); pkg/features/kube_features.go.

- A risky change starts behind a gate that is off by default.
- A feature that adds an API field starts in alpha.
- A gate that reached GA is locked to its default and is removed only after
  three releases in that state. A cluster that still passes the flag for a
  removed gate fails to start.
- Every gate has an entry in `defaultVersionedKubernetesFeatureGates` with its
  default for each version in which it changed. A change of stage adds an entry
  for the new version. It does not rewrite an older one: a control plane
  component can be set to behave as any of the three previous releases, and
  reads the older entry to do so.
- For the same reason the gate-off path stays in the code until the gate itself
  is removed. Flag a change that deletes it when the gate is only locked.
- When a gate is removed, the code keeps the path that ran with the gate on.
  Check each removed condition: dropping the wrong branch changes behavior for
  every cluster.

## Turning a gate off

Source: kubernetes/community: contributors/devel/sig-architecture/feature-gates.md
(Defining a feature gate, Disablement Tests).

- Disabling a gate must prevent further damage from bugs in the feature. Trace
  the gate-off state of every new behavior. Code added outside the gate check
  runs in every cluster.
- The effect of a gate stays within its feature. Flag a gate whose state changes
  what happens to workloads that do not use the feature.
- A gate can be turned off after it was on. Objects written in between still
  carry the fields of the feature. Check that the gate-off path reads such an
  object without a crash and without dropping the data.
- Tests for the disabled state stay until the gate is removed. After GA they set
  the emulation version to the release before GA with
  `SetFeatureGateEmulationVersionDuringTest`. Flag a change that deletes them
  while the gate still exists.

## Gates across components

Source: kubernetes/community: contributors/devel/sig-architecture/feature-gates.md
(Defining a feature gate, Node Declared Features).

- A gate is toggled in each component on its own, and no coordination between
  components is needed. A feature that spans the API server and the kubelet, or
  the API server and a controller, must work when only one side has it on. Name
  which side has the gate off, and what it does with the request or the object.
- When the control plane must know whether a node supports a feature, the
  feature uses the Node Declared Features framework instead of assuming the
  kubelet version.

## Gates in tests

Source: staging/src/k8s.io/component-base/featuregate/testing/feature_gate.go.

- A test sets a gate with `featuregatetesting.SetFeatureGateDuringTest`, which
  restores the value when the test ends. Direct writes to the gate are reported
  by `hack/verify-test-featuregates.sh`.
- The setting can leak when the test calls `t.Parallel()`. Flag `t.Parallel()`
  in a test, or in a parent of a test, that sets a gate.
