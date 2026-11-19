+++
title = "API Validation, Defaulting, and Registry Strategies"
paths = [
    "**/defaults.go",
    "**/strategy.go",
    "**/validation/",
    "pkg/api/",
    "pkg/registry/",
    "staging/src/k8s.io/apimachinery/pkg/api/validate/",
    "staging/src/k8s.io/code-generator/cmd/validation-gen/",
    "test/declarative_validation/",
]
symbols = [
    "DropDisabled*",
    "PrepareForCreate",
    "PrepareForUpdate",
    "dropDisabled*",
]
+++

# API Validation, Defaulting, and Registry Strategies

Applies to the code that decides which objects the API server accepts and what
it stores. Objects that were valid when they were stored stay in etcd, and every
later update of them runs the validation of the day.

## Changing the validation of an existing field

Source: kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(Backward compatibility gotchas, New enum value in existing field, Declarative
Validation); kubernetes/enhancements:
keps/sig-api-machinery/5073-declarative-validation-with-validation-gen/README.md
(Ratcheting).

- Validation rules on spec fields can neither be relaxed nor strengthened without
  a transition. A stricter rule rejects requests that worked. A weaker rule lets
  through values that other consumers of the resource do not expect.
- A stricter rule must not reject an update to a stored object that already
  breaks it. Check that the update path receives the old object and accepts a
  value that did not change. Name the stored object that the new rule rejects.
- A weaker rule, or a new allowed value, must not let the API server store data
  that the previous release would fail on. The transition takes two releases:
  the first accepts the new value only on update of an object that already has
  it, the second accepts it on create. Flag a new value that is accepted on
  create in the release that introduces it.
- A status field written only by controllers in this tree may be tightened.
- Validation never compares case-insensitively.
- Validation does not modify the object it checks, or the old object. The check
  in `hack/verify-non-mutating-validation.sh` is a text heuristic and misses
  most mutations.

## Fields of a disabled feature

Source: kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(Adding Unstable Features to Stable Versions); kubernetes/community:
contributors/devel/sig-architecture/feature-gates.md (Field wiping, Features
which change an existing API field); pkg/api/pod/util.go.

- When the gate is off, the strategy clears the field on create, and on update
  only if the old object does not already use it. Flag a new gated field with
  no such code: without it the field is stored while the feature is off.
- The field is cleared before validation runs, so that with the gate off the
  object is handled as if the field did not exist.
- The clearing is called from `PrepareForCreate` and `PrepareForUpdate` of every
  strategy that stores the type. That includes subresource strategies and every
  resource that embeds the type: `DropDisabledPodFields` and
  `DropDisabledTemplateFields` serve all resources that contain a pod.
- Keeping the field when the old object uses it is what preserves data across a
  gate that is turned off and across an API server one release older. Flag code
  that clears it without looking at the old object.
- A field that earlier releases always stored with a value is reset to that
  value, not to nil, or every stored object differs on the next write.
- A new allowed value for an existing field is gated too. The registry passes
  validation an option that allows the value when the gate is on or when the
  old object already has it.

## Declarative validation

Source: kubernetes/website:
content/en/docs/reference/using-api/declarative-validation.md; kubernetes/community:
contributors/devel/sig-architecture/feature-gates.md (Using a feature gate with
declarative validation); kubernetes/community:
contributors/devel/sig-architecture/api_changes.md (Declarative Validation).

- `+k8s:` tags on a type generate `zz_generated.validations.go`. Review the tag,
  not the generated function.
- Declarative validation of a new gated field does not check the gate. The field
  is cleared by the strategy instead. `+k8s:ifDisabled(...)=+k8s:forbidden` is
  for the cases that need stricter behavior.
- A declarative rule that tightens validation of an existing field is gated, so
  that it does not reject stored objects.
- A declarative rule and the hand-written rule it replaces must accept the same
  objects. After a rollback the hand-written rule decides again, and an object
  that only the declarative rule accepted can no longer be updated.
- A new rule for a list with `listType=map` must not depend on the order of the
  items.

## Defaulting

Source: kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(Edit defaults.go, Update the fuzzer).

- A field that gets a default is handled in the fuzzer function of its group,
  `pkg/apis/<group>/fuzzer/fuzzer.go`, so that defaulting does not change a
  fuzzed object again. Otherwise the round-trip tests fail at random.
- The round-trip fuzz test runs with no feature gate configuration. Defaulting
  that depends on a gate needs its own test for both states.
