+++
title = "API Types and Compatibility"
paths = [
    "pkg/apis/",
    "staging/src/k8s.io/api/",
    "staging/src/k8s.io/apiextensions-apiserver/pkg/apis/",
    "staging/src/k8s.io/apiserver/pkg/apis/",
    "staging/src/k8s.io/kube-aggregator/pkg/apis/",
    "staging/src/k8s.io/kube-controller-manager/config/",
    "staging/src/k8s.io/kube-proxy/config/",
    "staging/src/k8s.io/kube-scheduler/config/",
    "staging/src/k8s.io/kubelet/config/",
    "staging/src/k8s.io/metrics/pkg/apis/",
]
+++

# API Types and Compatibility

Applies to a change of an API type: a field, an enum value, a default, a
version. The hand-written sources are `types.go`, `defaults.go`, `conversion.go`,
and the validation package. The rest of an API package is generated.

## The compatibility contract

Source: kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(On compatibility).

- Any API call that succeeded before the change must succeed after it.
- It must be possible to roll back to an API server without the change. Ask what
  the previous release does with an object that this release stored.
- An object must convert to every other served version and back with no loss of
  information. A field added to one version needs a place in every version and
  in the internal type.

## Adding a field

Source: kubernetes/community: contributors/devel/sig-architecture/api-conventions.md
(Optional vs. Required, Serialization of optional/required fields, Primitive
types); kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(On compatibility, Edit types.go, Adding Unstable Features to Stable Versions).

- A new field carries an explicit `+optional` or `+required` tag. A field added
  to an existing version is optional: a compatible change does not add a new
  required field.
- An optional field has `omitempty` in its JSON tag. A field is read as required
  otherwise.
- A field of a new alpha feature in a stable version is optional, is a pointer
  or otherwise absent from responses when unset, and carries
  `+featureGate=<gate>`.
- Check how the zero value is told apart from "not set". A bool field is a
  pointer with `omitempty`. A required struct uses `omitzero`.
- Public integer fields are `int32` or `int64`, never unsigned.
- When an alpha field is abandoned, its name and protobuf tag are kept as a
  tombstone comment and never reused. When its schema must change incompatibly,
  it gets a new field name.

## Enum values and unions

Source: kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(Backward compatibility gotchas, New enum value in existing field).

- Adding a value to an enumerated set is not a compatible change. Old clients
  and old components read it and do not know it.
- A new value in a field that the kubelet reads must be handled safely by
  kubelets up to three minor versions older than the first API server that
  allows it.
- Adding a member to a union is acceptable. Removing one follows the deprecation
  process.

## Defaults

Source: kubernetes/community: contributors/devel/sig-architecture/api-conventions.md
(Defaulting); kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(On compatibility, Backward compatibility gotchas, Edit defaults.go).

- Only an optional field may have a default.
- A static default depends only on the object itself, and never replaces a value
  the user gave.
- A field with a default in one version has a non-nil default in every version.
  Defaulting runs on every read, so a default added to one version only makes
  the object differ after conversion.
- Changing the default of an existing field changes the meaning of every stored
  object that left it unset. It is not a compatible change.

## Versions and storage

Source: kubernetes/community: contributors/devel/sig-architecture/api_changes.md
(Backward compatibility gotchas); kubernetes/website:
content/en/docs/reference/deprecation-policy.md (Rule #1, Rule #4b).

- A new API version does not become the storage version in the release that
  introduces it. The previous release must be able to decode what is in etcd
  after a rollback.
- API elements are removed only by incrementing the version of the group. A
  field of a GA version is not removed.

## Lists and bounds

Source: kubernetes/community: contributors/devel/sig-architecture/api-conventions.md
(Validation); kubernetes/website:
content/en/docs/reference/using-api/server-side-apply.md (Merge strategy,
Compatibility across topology changes).

- A list field declares its `listType`, and `listMapKey` for a map list. Changing
  the list type of a released field changes how existing objects merge.
- Every string, list, and map field is checked for a maximum size, and every
  numeric field for both bounds. Flag a new field with no limit: it is stored,
  watched, and cached by every component.
