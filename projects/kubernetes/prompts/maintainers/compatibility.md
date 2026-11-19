+++
title = "Version Skew and Deprecation"
paths = [
    "cmd/",
    "pkg/controller/",
    "pkg/controlplane/",
    "pkg/kubeapiserver/",
    "pkg/kubelet/",
    "pkg/proxy/",
    "pkg/scheduler/",
    "staging/src/k8s.io/cri-api/",
    "staging/src/k8s.io/kube-controller-manager/",
    "staging/src/k8s.io/kube-proxy/",
    "staging/src/k8s.io/kube-scheduler/",
    "staging/src/k8s.io/kubectl/",
    "staging/src/k8s.io/kubelet/",
]
+++

# Version Skew and Deprecation

Applies to component code. A cluster is upgraded one component at a time, so
every pair of versions below exists in production for hours or for months. The
paths are wide on purpose: the rules apply to any code in a component.

## Supported version skew

Source: kubernetes/website: content/en/releases/version-skew-policy.md.

- `kube-apiserver` instances of one control plane are within one minor version
  of each other. During an upgrade the same object is written through the new
  server and read or updated through the old one.
- `kubelet` is never newer than `kube-apiserver`, and may be up to three minor
  versions older.
- `kube-proxy` is never newer than `kube-apiserver`, and may be up to three
  minor versions older. It may be up to three minor versions older or newer than
  the kubelet on its node.
- `kube-controller-manager`, `kube-scheduler`, and `cloud-controller-manager`
  are never newer than the API servers they talk to, and may be one minor
  version older.
- `kubectl` is within one minor version of `kube-apiserver`, older or newer.

Review a change against the pairs it crosses:

- New API server behavior must be safe for a kubelet and a kube-proxy three
  minor versions older, and for a controller manager and scheduler one minor
  version older. They ignore fields they do not know and may mishandle values
  they do not know.
- A component may rely on what an API server of its own version does. It may not
  rely on behavior that only a newer API server has, and a new `kubectl` may
  talk to a server one minor version older.
- A finding names the two versions, which side is older, and the request or
  object that crosses between them.

## Deprecation

Source: kubernetes/website: content/en/docs/reference/deprecation-policy.md
(Rule #5a, Rule #5b, Rule #7).

- A flag or other command line element of a user-facing component, such as
  `kubectl`, works after its announced deprecation for at least 12 months or 2
  releases when GA, and 3 months or 1 release when beta.
- For an administrator-facing component, such as `kubelet` or `kube-apiserver`,
  it is 6 months or 1 release when GA, and 3 months or 1 release when beta.
- A deprecated behavior functions for no less than 1 year after its announced
  deprecation.
- Flag a change that removes a flag, changes what it means, or changes a default
  behavior, with no deprecation before it. Use `git_log` on the file to find
  when the deprecation was announced.
