+++
title = "Authentication, Authorization, and Admission"
paths = [
    "pkg/apis/rbac/",
    "pkg/auth/",
    "pkg/certauthorization/",
    "pkg/controller/certificates/",
    "pkg/credentialprovider/",
    "pkg/kubeapiserver/",
    "pkg/registry/rbac/",
    "pkg/serviceaccount/",
    "plugin/pkg/admission/",
    "plugin/pkg/auth/",
    "staging/src/k8s.io/apiserver/pkg/admission/",
    "staging/src/k8s.io/apiserver/pkg/authentication/",
    "staging/src/k8s.io/apiserver/pkg/authorization/",
    "staging/src/k8s.io/apiserver/plugin/pkg/",
    "staging/src/k8s.io/client-go/plugin/pkg/client/auth/",
    "staging/src/k8s.io/kms/",
    "staging/src/k8s.io/pod-security-admission/",
]
symbols = [
    "SubjectAccessReview",
]
+++

# Authentication, Authorization, and Admission

Applies to the code that decides who a request comes from and what it may do.
Kubernetes has few written rules for this code. Where a rule below comes from
a comment or a test in the tree, the source says so, and the tree at the target
commit decides.

## RBAC and bootstrap policy

Source: plugin/pkg/auth/authorizer/rbac/bootstrappolicy/policy_test.go;
plugin/pkg/auth/authorizer/rbac/bootstrappolicy/policy.go; kubernetes/website:
content/en/docs/concepts/security/rbac-good-practices.md.

- The default roles and bindings are compared with the YAML fixtures under
  `plugin/pkg/auth/authorizer/rbac/bootstrappolicy/testdata/`. A change of
  policy comes with a change of the fixtures in the same commit. Read the
  fixture diff: it is the list of rights that every cluster gains or loses.
- A new rule in a default role is given to every holder of that role in every
  cluster. Name who gets the right and what they can do with it.
- Some rights are wider than they look. `get` on `nodes/proxy` is not read-only.
  The `escalate`, `bind`, and `impersonate` verbs let the holder exceed their
  own rights. Read access to secrets, and access to `pods/exec`, `pods/attach`,
  `pods/portforward`, `pods/proxy`, and `services/proxy`, are escalating. Flag
  one of these added to a role that ordinary users or workloads hold.

## Node identity

Source: kubernetes/website: content/en/docs/reference/access-authn-authz/node.md;
kubernetes/website:
content/en/docs/reference/access-authn-authz/admission-controllers.md
(NodeRestriction).

- A kubelet reads only its own Node, the pods bound to its node, and the objects
  those pods refer to. It modifies only its own Node and the pods bound to it.
- A change to the node authorizer or to `NodeRestriction` that widens this lets
  one compromised node act on the rest of the cluster. Check each new rule for
  how it ties the object to the requesting node.

## Admission plugins

Source: pkg/kubeapiserver/options/plugins.go (comments in the plugin order
list).

- The order of admission plugins is fixed in `AllOrderedPlugins`. New plugins go
  above the marked line. Webhook, resource quota, and deny plugins stay at the
  end, so that they see the object after every other mutation.
- A new plugin does not configure itself through command line flags.

## Delegated authorization

Source: kubernetes/website:
content/en/docs/tasks/extend-kubernetes/configure-aggregation-layer.md
(Authentication Flow).

- An extension API server authorizes the original user of a request, with a
  SubjectAccessReview to the kube-apiserver. Code that acts for a user checks
  that user's rights, not the rights of the component that forwards the request.

## Values that users write

Source: kubernetes/community: contributors/devel/sig-architecture/api-conventions.md
(Representing Allocated Values, Object references).

- The system does not trust users with fields that represent allocated values.
  It verifies or confirms such a request before it uses it.
- A component does not modify the object that a reference points to. The user
  who wrote the reference may have no right to change the target.
