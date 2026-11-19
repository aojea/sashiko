# Severity Levels

Assign a severity to every finding. Take it seriously: `Critical` must be
reserved for catastrophic failures, and `High` must mean severe functional
breakage. Use `Medium` as the default for real functional defects and `Low` for
defensive hardening, rare edge cases, and hygiene.

Kubernetes runs production clusters that many tenants share and that are
upgraded in place. Calibrate against *those* consequences, not against
theoretical perfection.

## Calibrating the level (reason before you label)

State this reasoning at the start of `severity_explanation` so the label is
auditable.

- **Blast radius and irreversibility**: What actually happens when the bug
  triggers? Stored objects that no API server can read again are catastrophic
  (`Critical`); a wrong condition on one object that the next sync corrects is
  contained (`Medium`). Ask whether one object, one node, one namespace, or the
  whole cluster is affected, and whether a restart or the next resync repairs it.
- **Who controls the trigger**: Name the identity. The Kubernetes Security
  Response Committee rates an attack lower when it needs more access:
  anonymous and remote is the worst case, an authenticated user with ordinary
  rights is next, and a cluster administrator harming their own cluster is not
  an attack. A defect that only an administrator's own configuration can
  trigger is a functional defect, not a security one.
- **Likelihood and preconditions**: Name the path and what it needs: a feature
  gate, a flag, a version skew, a cluster size, or a race window. A defect on
  the default path of a core component deserves a higher level than one that
  needs an uncommon setting or a narrow window.
- **Feature gate state**: A defect that exists only while an alpha gate that is
  off by default is enabled is capped at `Medium`. Kubernetes does not backport
  such fixes. The cap does not apply when the defect changes behavior with the
  gate off, breaks the gate-off path, or leaves stored data that harms the
  cluster after the gate is turned off again.
- **Speculative findings**: If a finding rests on an unverified assumption, such
  as how a container runtime or a cloud provider behaves, mark it speculative
  and cap its severity at `Medium` (or `Low` if the impact is minor).

Source: kubernetes/committee-security-response: severity-ratings.md (Severity
Thresholds - Heuristics); kubernetes/community:
contributors/devel/sig-release/cherry-picks.md (What Kind of PRs are Good for
Cherry Picks).

## Critical

- **Definition**: An identity that gets past a trust boundary meant to contain
  it, or loss or corruption of stored cluster state that an upgrade, a rollback,
  or a restart does not repair.
- **Question to ask**: Can an anonymous client, an ordinary authenticated user,
  or a node actually reach this, and does it give them the rights of another
  identity, the content of secrets they may not read, or code execution on a
  control plane host or another node? Or does it destroy stored objects beyond
  repair? If no, it is **not** `Critical`.
- **Examples**:
  - An authorization or admission check that a request can skip, so that a user
    reads or writes objects outside their rights.
  - A kubelet credential that can read secrets of pods on other nodes, or modify
    another Node.
  - A user-written object field used as a host path, a command argument, or an
    address by a privileged component without validation.
  - A storage or conversion change after which the API server cannot decode
    objects already in etcd, or drops their fields on the next write.

## High

- **Definition**: Crash, deadlock, data loss, or workload disruption on a
  realistic path, or a compatibility break, without the reach of a `Critical`
  finding.
- **Question to ask**: Would a cluster with a supported configuration hit this
  during ordinary operation, an upgrade, or a rollback?
- **Examples**:
  - A nil dereference or unrecovered panic on a normal path of a control plane
    component or the kubelet.
  - A deadlock, or a control loop that stops processing its queue.
  - A validation change that rejects updates to objects that are already stored.
  - A change that breaks a supported version skew, such as a field that a
    kubelet three minor versions older mishandles.
  - Running pods killed, evicted, or left without network or storage because of
    a wrong decision.
  - A request that one authenticated user can send to make the API server
    unavailable to others.
  - A defect in a security feature, such as a rule that is not enforced.

## Medium

- **Definition**: A real functional defect with contained impact. This is the
  default level for a verified bug.
- **Examples**:
  - A goroutine, watch, or timer that leaks on an error path.
  - An object from an informer cache mutated without a deep copy.
  - A failed sync that is dropped instead of requeued, when a later resync
    recovers it.
  - A hot loop: a requeue without backoff, or an API call per object in a
    control loop.
  - A metric with a label whose values are not bounded.
  - A race with a narrow window and a recoverable result.
  - A wrong status or condition that the next sync corrects.
  - A defect present only while an off-by-default alpha gate is enabled.

## Low

- **Definition**: Defensive hardening, rare edge cases, and hygiene.
- **Examples**:
  - A missing check that no caller in the tree can trigger.
  - A log message or an error message that misleads.
  - A commit message that does not follow the commit message guidelines.
  - A test that sleeps for a fixed time, or missing test coverage for a change
    that the conventions expect to have it.
  - A comment or a field description that disagrees with the code.
