+++
title = "Kubelet and the Container Runtime Interface"
paths = [
    "cmd/kubelet/",
    "pkg/kubelet/",
    "staging/src/k8s.io/component-helpers/nodedeclaredfeatures/",
    "staging/src/k8s.io/cri-api/",
    "staging/src/k8s.io/cri-client/",
    "staging/src/k8s.io/kubelet/",
    "test/e2e_node/",
]
+++

# Kubelet and the Container Runtime Interface

Applies to the node agent and to the API it uses to drive container runtimes.
The kubelet is restarted and upgraded while its pods keep running, it is the
component with the widest version skew, and its container runtime is not in
this tree.

## Pod workers

Source: kubernetes/community: contributors/devel/sig-node/kubelet.md (Startup
Flow: From API to Pod Worker, Runtime Manager Sync).

- Each pod has its own worker goroutine, so operations on one pod are
  serialized. Operations on different pods run at the same time. State that
  several pods or several managers share needs its own lock. Name the two
  goroutines before reporting a race.
- A PostStart hook runs synchronously and blocks the other containers of its
  pod. Work added to the start path of a container delays every container that
  starts after it.

## Container Runtime Interface

Source: kubernetes/contributor-site:
content/en/docs/code/cri-api-version-skew-policy.md (Version Skew Policy for CRI
API).

- Every change to the CRI API within `v1` is backward and forward compatible.
- The kubelet works with a container runtime that implements the CRI API of up
  to three Kubernetes minor versions back. A new feature degrades gracefully:
  the kubelet handles a runtime that does not set a new field or does not
  implement a new call.
- The kubelet works with a runtime up to three minor versions ahead. A new
  feature in the CRI API does not change the behavior of existing calls or the
  meaning of existing response values.
- Flag kubelet code that treats a new CRI field as always set, or that fails a
  pod when a new call returns an unimplemented error, unless the feature is
  explicitly requested and cannot work without it.

## What a kubelet may reach

Source: kubernetes/website: content/en/docs/reference/access-authn-authz/node.md;
kubernetes/website:
content/en/docs/reference/access-authn-authz/admission-controllers.md
(NodeRestriction).

- The node authorizer and the `NodeRestriction` admission plugin limit what a
  kubelet credential can read and write. A change that makes the kubelet read or
  write another kind of object, or a new part of a Node or a Pod, must fit
  inside those limits. Check that the code in
  `plugin/pkg/auth/authorizer/node/` and
  `plugin/pkg/admission/noderestriction/` at the target commit allows exactly
  what the change needs. Flag a change that only works with wider node
  permissions.

## Node declared features

Source: kubernetes/community:
contributors/devel/sig-node/node-declared-features.md (When to add a declared
feature, Adding a declared feature, Lifecycle).

- `Discover` returns false when any feature gate listed in `Requirements()` is
  disabled.
- A declared feature is for a feature that is still moving toward GA. Do not
  declare one that is already beta or GA, or one that describes a permanent
  attribute of the node.
- Declared features do not accumulate. An entry sets `MaxVersion` and is deleted
  when its gate is removed.
