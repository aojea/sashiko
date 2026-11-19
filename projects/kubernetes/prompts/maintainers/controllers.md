+++
title = "Controllers, Informers, and Workqueues"
paths = [
    "cmd/kube-controller-manager/",
    "pkg/controller/",
    "staging/src/k8s.io/client-go/tools/cache/",
    "staging/src/k8s.io/client-go/tools/leaderelection/",
    "staging/src/k8s.io/client-go/util/retry/",
    "staging/src/k8s.io/client-go/util/workqueue/",
    "staging/src/k8s.io/cloud-provider/",
    "staging/src/k8s.io/controller-manager/",
    "staging/src/k8s.io/endpointslice/",
    "staging/src/k8s.io/sample-controller/",
]
symbols = [
    "AddAfter",
    "AddEventHandler",
    "AddEventHandlerWithResyncPeriod",
    "AddRateLimited",
    "DeletedFinalStateUnknown",
    "DeletionHandlingMetaNamespaceKeyFunc",
    "DeletionHandlingObjectToName",
    "Finalizers",
    "GetControllerOf",
    "IsControlledBy",
    "OwnerReferences",
    "RateLimitingInterface",
    "ResourceEventHandlerFuncs",
    "RetryOnConflict",
    "SharedIndexInformer",
    "SharedInformerFactory",
    "TypedRateLimitingInterface",
    "UpdateStatus",
    "WaitForCacheSync",
    "WaitForNamedCacheSync",
    "WaitForNamedCacheSyncWithContext",
    "leaderelection",
    "workqueue",
]
+++

# Controllers, Informers, and Workqueues

Applies to any control loop built on client-go informers and workqueues,
wherever it lives. None of these rules is checked by `make verify`.

## Level-driven logic

Source: kubernetes/community: contributors/devel/sig-api-machinery/controllers.md
(Guidelines); kubernetes/community:
contributors/devel/sig-architecture/api-conventions.md (Sequencing operations).

- A controller acts on the state it observes now. It cannot count on having seen
  a value change: it may have been down, or the informer may have merged two
  updates. Flag logic that works only if a specific event is delivered.
- A delete and a create of the same key can arrive as one update. Code that must
  tell them apart compares `ObjectMeta.UID` in the update handler.
- A sync is idempotent and safe after a crash at any point. What a controller
  must remember is stored in the object's status, not in memory.
- There are other actors. Absence from the cache does not prove that an object
  does not exist, and the cache can lag the controller's own writes.

## Objects from the cache are read-only

Source: staging/src/k8s.io/client-go/tools/cache/controller.go
(ResourceEventHandler); staging/src/k8s.io/client-go/tools/cache/thread_safe_store.go;
kubernetes/community: contributors/devel/sig-api-machinery/controllers.md
(Guidelines).

- Objects from a lister, an indexer, or an event handler are shared with the
  cache and with every other reader. Never modify them, at any depth. Call
  `DeepCopy()` first.
- Follow a cached object through helpers. A helper that sets a field, appends to
  a slice it was given, or sorts it in place modifies the cache.

## Caches and handlers

Source: staging/src/k8s.io/client-go/tools/cache/shared_informer.go;
staging/src/k8s.io/client-go/tools/cache/controller.go (ResourceEventHandler);
kubernetes/community: contributors/devel/sig-api-machinery/controllers.md (Rough
Structure).

- A controller waits for every informer it reads, with `WaitForCacheSync` or a
  named variant, before it starts its workers.
- An event handler returns quickly. It computes a key and adds it to the queue.
  A handler that blocks delays every later notification.
- A delete handler can receive a `DeletedFinalStateUnknown` tombstone instead of
  the object. It handles both, for example with
  `DeletionHandlingMetaNamespaceKeyFunc` or `DeletionHandlingObjectToName`. The
  object inside a tombstone may be stale.

## Workqueue contract

Source: staging/src/k8s.io/client-go/util/workqueue/queue.go;
staging/src/k8s.io/client-go/util/workqueue/rate_limiting_queue.go;
kubernetes/community: contributors/devel/sig-api-machinery/controllers.md (Rough
Structure).

- Every `Get` is paired with `Done` for the same item, on every path. A worker
  exits when `Get` reports shutdown.
- On success the worker calls `Forget`, and still calls `Done`. Without `Forget`
  the rate limiter tracks the failures of that item forever.
- On failure the item is added again with `AddRateLimited` or `AddAfter`. A
  plain `Add` on failure is a hot loop.
- The queue is shut down when the controller stops, and each worker goroutine
  recovers from a panic with `HandleCrash` or `HandleCrashWithContext`.

## Errors and requeue

Source: kubernetes/community: contributors/devel/sig-api-machinery/controllers.md
(Guidelines).

- The sync function returns the error when the item needs a retry, so that the
  item is requeued. Flag a path that logs an error and returns nil when the work
  is not done: the item is never retried.
- Do not skip a requeue after a failure for a reason such as an unchanged
  `resourceVersion`.

## Conflicts and status

Source: kubernetes/community: contributors/devel/sig-architecture/api-conventions.md
(Concurrency Control and Consistency, Spec and Status);
staging/src/k8s.io/client-go/util/retry/util.go (RetryOnConflict).

- After a conflict the client reads the object again, applies the change again,
  and submits. The function passed to `retry.RetryOnConflict` fetches the object
  on every attempt and returns the update error unwrapped. A read from a cache
  that lags does not resolve a conflict.
- `resourceVersion` is opaque. Do not parse it, compare it by order, or set it
  by hand.
- Status is written through the status subresource. An update of the main
  resource ignores status.
- A controller that reports progress sets `status.observedGeneration` from
  `metadata.generation`.
- A controller that sets a field the user left empty uses a patch or apply, so
  that it does not overwrite unrelated fields.

## Ownership, deletion, and finalizers

Source: kubernetes/community: contributors/devel/sig-api-machinery/controllers.md
(Guidelines); kubernetes/community:
contributors/devel/sig-architecture/object-lifecycle.md (Handling DELETING);
staging/src/k8s.io/apimachinery/pkg/apis/meta/v1/types.go (ObjectMeta);
pkg/controller/controller_ref_manager.go; kubernetes/website:
content/en/docs/concepts/overview/working-with-objects/finalizers.md.

- A controller decides what to do with an object that has a
  `deletionTimestamp`. Flag a sync that treats it like a live object.
- A controller does not adopt children when the parent or the child is being
  deleted, and reads the parent from the API server, not from the cache, before
  it adopts.
- Ownership is tested with `metav1.IsControlledBy`, which compares the UID. An
  object has at most one owner reference with `controller: true`, and an owner is
  in the same namespace or cluster-scoped.
- A finalizer is not added after `deletionTimestamp` is set. Finalizers have no
  order. An object stays in deletion until its last finalizer is removed, so
  check that every finalizer a controller adds has a path that removes it.

## Leader election

Source: staging/src/k8s.io/client-go/tools/leaderelection/leaderelection.go.

- Leader election does not guarantee that only one client acts as leader. A
  controller stays correct when two instances run for a short time.
- With `ReleaseOnCancel`, the context is cancelled only after all work guarded by
  the lease has finished.

## Data from referenced objects

Source: kubernetes/community: contributors/devel/sig-architecture/api-conventions.md
(Object references).

- A controller does not copy values from a referenced object into the object
  that refers to it, into other objects, or into logs and events. The user who
  wrote the reference may not be allowed to read the target.
- A field that is used as a path segment in an API request is validated first.
  A value with `..` or `/` reaches a different resource.
- Built-in types do not refer across namespaces.
