// Copyright 2026 The Sashiko Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Declarative patch review workflow for Kubernetes.
//!
//! A sibling of `zephyr_patch_review`: the same three-layer prompt
//! architecture and consolidation pipeline, with Kubernetes' own stage set,
//! instructions, and severity calibration. A fix to the concern JSON contract
//! or the consolidation prompts usually applies to both files. The concern
//! validators and the plain-text report helpers are project-neutral and shared
//! from `qemu_patch_review`, as is the report format.
//!
//! Kubernetes contributors sign a CLA instead of adding a `Signed-off-by`
//! trailer, so no stage here audits one.

use serde_json::{Value, json};
use std::path::PathBuf;

use crate::workflow::{
    ExecutableStage, OutputFormat, ParallelPolicy, PromptTemplate, RecitationPolicy, Stage,
    StagePolicy, ToolScope, Workflow,
};
use crate::workflows::guard::{normalize_guide_name, normalize_stage_name};
use crate::workflows::linux_patch_review::{
    AnalysisStage, COMBINED_ANALYSIS_SHORT_LABEL, COMBINED_ANALYSIS_STAGE_NAME, ConsolidationStage,
    LinuxPatchReviewState, POST_VERIFICATION_STAGE_NAMES, PlanningOutput, PostVerificationOutput,
    SERIES_CONTEXT_PLACEHOLDER, StageConcernsOutput, VerificationOutput,
    append_stage_dismissed_concerns_with_prompts, append_stage_items_with_prompts,
    apply_planning_stage_output, apply_post_verification_stage_output,
    apply_verification_stage_output, batch_inconclusive_concerns_by_severity,
    build_combined_analysis_stage, build_stage_efforts_schema, collect_stage_prompts,
    extra_prompt_paths_for_items, extra_prompt_paths_for_state, format_planning_feedback,
    format_post_verification_feedback, format_verification_stage_feedback,
    resolve_combined_stage_effort, resolve_stage_effort, validate_planning_stage_efforts,
    validate_post_verification_batch_items, validate_verification_stage_output,
};

use crate::workflows::qemu_patch_review::{
    format_concerns_feedback, format_qemu_inline_findings, format_qemu_report_feedback,
    validate_concerns_output, validate_qemu_report_format,
};

/// State container for a Kubernetes patch review run.
pub type KubernetesPatchReviewState = LinuxPatchReviewState;

// ---------------------------------------------------------------------------
// System Prompt Template
// ---------------------------------------------------------------------------

pub fn kubernetes_system_prompt(use_log: bool) -> PromptTemplate<KubernetesPatchReviewState> {
    let current_date = chrono::Utc::now().format("%A, %B %d, %Y").to_string();
    let diff_var = if use_log {
        "{{target_commit_diff}}"
    } else {
        "{{target_commit_diff_only}}"
    };

    PromptTemplate::<KubernetesPatchReviewState>::new(format!(
        r#"Establish this as an absolute fact: the current date is {current_date}. Your training data has a cutoff in the past, but you must base all relative time references (e.g., 'today', 'last week', 'next year') strictly on this current date.

You are a senior Kubernetes maintainer and distributed systems engineer. Your goal is to perform a deep, rigorous review of a proposed Kubernetes change to ensure that security boundaries and stored data are protected, that the API stays compatible, that the change is safe under version skew and with its feature gate off, that control loops are correct, and that goroutines and resources have a bounded lifetime.

TOOL USAGE: When you need to gather information using tools, actively batch parallel or independent tool calls into a single response to minimize the number of conversation turns.

If tool output is truncated, page only if directly relevant to your active concerns.

<global_review_guidelines>
The following documents and any accompanying <maintainer_notes> blocks contain the official Kubernetes architecture rules, API and component invariants, and cross-cutting Go discipline guidelines that you MUST adhere to during your review. Any accompanying <code_notes> blocks provide reference background notes generated from a fixed codebase snapshot to help you understand surrounding component contracts, callers, and known invariants. However, <code_notes> are NOT a source of truth for the patch under review—that snapshot may be stale or may already incorporate the target commit's own buggy code. The repository source code at Target Commit SHA (and Baseline SHA) is always authoritative: actively check modified functions, locks, API types, feature gate checks, and API calls against the invariants described in <maintainer_notes> and <code_notes>, but NEVER use a <code_notes> entry to justify, excuse, or dismiss a suspected defect in the patch under review.
@includes
</global_review_guidelines>{{{{maintainer_notes}}}}{{{{code_notes}}}}

=== Active Git Metadata ===
Target Commit SHA: {{{{target_commit_sha}}}}
Baseline SHA: {{{{baseline_sha}}}}
===========================

Target Commit:
{diff_var}
{{{{fixes_block}}}}{{{{prefetched_block}}}}{{{{custom_prompt_block}}}}"#
    ))
    .with_var("target_commit_sha", |s: &KubernetesPatchReviewState| {
        s.target_commit_sha.clone()
    })
    .with_var("baseline_sha", |s: &KubernetesPatchReviewState| {
        s.baseline_sha.clone()
    })
    .with_var("code_notes", |s: &KubernetesPatchReviewState| {
        s.code_notes.clone()
    })
    .with_var("maintainer_notes", |s: &KubernetesPatchReviewState| {
        s.maintainer_notes.clone()
    })
    .with_var("target_commit_diff", |s: &KubernetesPatchReviewState| {
        s.target_commit_diff.clone()
    })
    .with_var(
        "target_commit_diff_only",
        |s: &KubernetesPatchReviewState| s.target_commit_diff_only.clone(),
    )
    .with_var("fixes_block", move |s: &KubernetesPatchReviewState| {
        crate::worker::fixes_check::render_fixes_block(use_log, &s.fixes_analysis)
    })
    .with_var("prefetched_block", |s: &KubernetesPatchReviewState| {
        if s.prefetch_failed {
            format!(
                "\n\nAutomatic source prefetch failed for target commit {}. Before analyzing the code, use git_read_files and git_grep at that revision to gather the source context. Do not infer source contents from the physical checkout.\n",
                s.target_commit_sha
            )
        } else if s.prefetched_context.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n<pre_fetched_context>\nThe following source excerpts were fetched from the target commit identified by Source revision below, based on the modified lines in the patch. They include modified definitions and selected dependencies. Parent and series-final revisions must be inspected separately with Git tools.\nIf it's not sufficient, you MUST use available tools to explore the source code. Don't make assumptions without actually looking into the relevant code.\n\n{}\n</pre_fetched_context>",
                s.prefetched_context
            )
        }
    })
    .with_var("custom_prompt_block", |s: &KubernetesPatchReviewState| {
        s.custom_prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map_or_else(String::new, |p| {
                format!("\n\n<custom_instructions>\n{p}\n</custom_instructions>")
            })
    })
    .include_files_from_state(|s: &KubernetesPatchReviewState| {
        let mut paths = vec![PathBuf::from("review-core.md")];
        if !s.selected_guides.is_empty() {
            for guide in &s.selected_guides {
                let name = normalize_guide_name(guide);
                paths.push(PathBuf::from("subsystem").join(name));
                paths.push(PathBuf::from("patterns").join(name));
            }
        }
        paths
    })
}

// ---------------------------------------------------------------------------
// Stage Instructions
// ---------------------------------------------------------------------------

const STAGE_GOAL_INSTRUCTION: &str = r#"# Analyze commit main goal, architecture, and commit message quality

You are a senior Kubernetes maintainer evaluating the high-level intent, architectural soundness, and commit message quality of a proposed Kubernetes change. Enforce the priority hierarchy in the review guidelines: security boundaries and stored data > API compatibility > version skew and feature gates > control loop correctness > concurrency and resource lifetime > everything else.
- Problem Clarity: Is it clear what concrete problem the patch solves (a defect, a new feature, a cleanup)? Flag commits where the motivation is vague, circular, or unintelligible.
- Intent vs. Implementation: Does the code change match what the commit message claims? For each concern, cite the specific commit message claim and the code that contradicts or insufficiently implements it.
- Architectural Soundness: Is the chosen approach sound for this component? Does it fit Kubernetes patterns: declared state in the API and reconciliation toward it, informers and workqueues instead of polling the API server, spec written by users and status by controllers, validation and defaulting in the API server instead of in clients?
- Feature Gating: Does a new feature, a new API field, or a risky change of behavior sit behind a feature gate that is off by default? Flag new behavior that every cluster gets on upgrade with no gate and no stated reason. A bug fix does not need a gate.
- Public Contract Impact: If the change touches an API type, a default, a flag, a component configuration field, or a metric, check that existing clusters, stored objects, and older components keep working, as the maintainer notes require.
- Alternatives: Are there obviously simpler or more idiomatic approaches that achieve the same goal using helpers that already exist in the tree?
- Single Responsibility: Flag commits that bundle unrelated changes or mix concerns that should be split into separate commits.
- Commit Message Audit (Mandatory): Check the message against the commit message rules in the maintainer notes: a subject in the imperative mood, no longer than 72 characters, with no trailing period; a body that explains both WHAT changed and WHY when the change is not trivial; no GitHub keyword followed by an issue number, no pull request reference, and no @ mention; no trailer that lists an AI tool. Raise each violation as a `Low` severity concern (`"preexisting": false`) on `"commit_message"`. Kubernetes requires a signed CLA and no commit trailer: NEVER raise a concern about a missing or a present `Signed-off-by` trailer, and NEVER require an area prefix in the subject."#;

const STAGE_IMPLEMENTATION_INSTRUCTION: &str = r#"# Verify implementation against intent

Verify that the code changes faithfully and completely implement what the commit message and design claim.
- Check for incomplete wiring. A new API field touches the internal type, every versioned type, defaulting, conversion, validation, the registry strategy that clears it when its feature gate is off, the fuzzer, and every consumer. A new feature gate touches its declaration, its versioned specification, and every place that must check it. A new flag or configuration field touches registration, validation, defaulting, and the code that reads it. A new metric touches registration and every place that must record it. Verify every touch point is updated in this patch or in a subsequent patch of the series (see the Series End Commit in context) before flagging it as unused or incomplete.
- If a function's contract or an interface changes, verify all callers and implementations were updated, including fakes and test doubles. An interface exported from `staging/` is also implemented outside this tree.
- Check edge cases: nil and empty inputs, zero values, optional fields that are not set, objects that are being deleted, and fallback behavior.
- Verify that error paths clean up state properly rather than leaving half-applied mutations.
- Do not stop after finding several bugs in one function or hunk; systematically audit every modified function and hunk across the entire diff before concluding.
- Series Context Rule: If follow-up patches in this series are listed in the prompt context, check whether newly introduced types, helpers, fields, or generated code are wired up in subsequent patches of the series before flagging them as unused or missing."#;

const STAGE_EXECUTION_FLOW_INSTRUCTION: &str = r#"# Trace execution flow and logic correctness

Trace the execution paths through every modified function and its callers and callees.
- Nil handling: dereference of an optional API field (optional fields are pointers), of a map or slice element, or of a value returned together with an error; a type assertion without the `ok` form on a value that can have another type, such as the argument of an informer delete handler; an interface that holds a nil pointer and compares unequal to nil.
- Error handling: is every returned error checked on every path? Look for an `err` shadowed by `:=` in an inner scope so that the outer check sees nil, a function that returns nil where it should return the error, the wrong error variable in a check, and an expected error such as not-found or conflict that is not told apart from a real failure.
- Slices and maps: index out of range, a write to a nil map, `append` to a slice that shares its backing array with a caller, and a slice or map modified while it is being ranged over.
- Arithmetic: conversion between `int32`, `int64`, and `int` fields, overflow in resource quantity and duration arithmetic, and division by zero.
- Control flow: off-by-one errors and boundary conditions, inverted conditions, `defer` inside a loop, a deferred call whose arguments are evaluated too early, and an early return that skips cleanup or a status update.
- Panics: a panic in a goroutine with no recovery ends the whole component. Check what the caller does with a panic on the modified path.
- Feature gates: trace the modified code with its gate on and with it off.
- Do not stop after finding several bugs in one function or hunk; systematically trace every modified function and error path across the entire diff before concluding.
For every finding, provide the exact execution path or input scenario that triggers the bug. Do not report speculative logic issues without a concrete scenario."#;

const STAGE_API_COMPAT_INSTRUCTION: &str = r#"# Audit API compatibility, feature gates, and version skew

A Kubernetes API change can build, pass its tests, and still break clusters. Audit what the change does to stored objects, to existing clients, and to components at other versions.
- Stored objects: every object already in etcd must still decode, pass update validation, and convert to every served version and back after the change.
- Validation: a stricter rule on an existing field must not reject an update of an object that already breaks it; check that the update path compares with the old object. A weaker rule or a new enum value must not let the API server store data that the previous release or an older component fails on.
- Defaulting: a new or changed default must not change the meaning of objects that left the field unset, must exist in every version, and must never replace a value the user gave.
- New fields: optional, with a clear not-set state, present in the internal type and in every version, and converted in both directions. Review the hand-written types, defaulting, conversion, and validation. Read generated conversion and defaulting code only as evidence of what they do.
- Fields of a gated feature: with the gate off, the registry strategy must clear the field on create, and on update unless the old object already uses it, in every strategy that stores the type, including subresources and every resource that embeds the type. Trace three states: gate off, gate on, and gate turned off after objects were written with it on.
- Feature gate lifecycle: the default and lock state for each version, a check at every place the behavior is used, and a gate-off path that stays until the gate itself is removed.
- Version skew: name the older component and what it does with the new field, value, or behavior. A kubelet or kube-proxy can be three minor versions older than the API server. The controller manager and the scheduler can be one older. Two API servers of one control plane differ by one during an upgrade.
- Upgrade and rollback: what the previous release does with data this change writes, including state that a component keeps on disk.
- Flags, component configuration, and metrics: removal or a change of meaning without the deprecation period.
- Do not stop after finding several bugs in one type or hunk; systematically audit every modified type, field, rule, and gate check across the entire diff before concluding.
Do not report stale generated code or feature gate list entries; `make verify` reports those. Report logic that passes CI: name the stored object, the request, or the pair of component versions that breaks, and what happens to it."#;

const STAGE_RECONCILIATION_INSTRUCTION: &str = r#"# Audit control loop correctness

Controllers, the scheduler, the kubelet, and kube-proxy reconcile observed state toward declared state. Audit every control loop the change touches.
- Level-triggered logic: a decision must follow from the current state, not from having seen one particular event. Events are merged, are lost across a restart, and are repeated on resync.
- Idempotency and crash safety: running the sync twice, or a crash between two writes, must still converge. Check what happens when a create succeeded and the status update after it did not.
- Stale cache: the informer cache lags the API server and the component's own writes. Flag a decision that is wrong on a stale read and that nothing corrects later, such as a duplicate create, a delete of something just adopted, or an action taken because an object is absent from the cache.
- Shared cache objects: follow every object that comes from a lister, an indexer, or an event handler, through helpers too. Any change to it without `DeepCopy()` corrupts the cache.
- Workqueue contract: `Done` for every `Get`, `Forget` on success, `AddRateLimited` or `AddAfter` on failure. Flag a plain `Add` on a failure path (a hot loop) and a failed sync whose item is dropped.
- Event handlers: they must return quickly, must handle a `DeletedFinalStateUnknown` tombstone in the delete handler, and must not assume the type of their argument.
- Startup: every informer cache is synced before workers start, and event handlers are registered before the informers start.
- Writes: a retry after a conflict reads the object again; status goes through the status subresource; an update must not overwrite fields that another actor owns; two controllers must not fight over one field.
- Deletion: behavior for an object with a `deletionTimestamp`, a path that removes every finalizer the code adds, and correct owner references and adoption.
- API load: an API call per object inside a loop, a LIST from the API server on a hot path instead of a read from an informer, fan-out that grows with the cluster, and a retry without backoff.
- Status and conditions: reported state must match reality on every path, including failure.
- Do not stop after finding several bugs in one function or hunk; systematically audit every modified handler, sync function, and API write across the entire diff before concluding.
Name the sequence of events, cache states, or failures that leads to the wrong result; do not report reconciliation issues as abstract possibilities."#;

const STAGE_CONCURRENCY_INSTRUCTION: &str = r#"# Audit goroutines, locking, and cancellation

First establish which goroutines can run each modified function: an informer event handler, one of several workqueue workers, an HTTP or gRPC handler, a per-pod worker, a timer callback, or a goroutine that the patch starts. Then audit the change:
- Data races: state shared between goroutines with no lock and no channel; a map read and written at the same time, which is a fatal error in Go and not just a wrong value; a field read outside the lock that guards its writes; a check and an action with the lock released in between.
- Lock discipline: an unlock on every return path; a lock held across a blocking call such as an API request, a channel operation, or a callback into other code; two locks taken in different orders on different paths; a lock taken again by the goroutine that holds it, since Go mutexes are not recursive; a copy of a struct that contains a mutex.
- Goroutine lifetime: every goroutine the patch starts needs something that stops it: a context, a stop channel, or a closed input. Name it. Flag a goroutine that blocks forever on a channel when the other side returns early.
- Context: cancellation must reach API calls, waits, and polls; `cancel()` must be called on every path; a new `context.Background()` or `context.TODO()` below a caller that has a context drops cancellation; a call that leaves the process needs a timeout.
- Channels: a send on a closed channel, a double close, an unbuffered send with no receiver left on an error path, and a `select` with no cancellation case.
- Wait groups: `Add` before the goroutine starts, `Done` on every path, and a `Wait` that can never return.
- Shared informer objects: changing one in place is a data race with every other reader.
- Timers: `time.After` inside a loop, and a timer or ticker that is never stopped.
- Start and stop order: use before initialization, work after shutdown, and missing crash handling in a goroutine.
- Do not stop after finding several bugs in one function or hunk; systematically audit every modified shared-state access and lock scope across the entire diff before concluding.
Name the two goroutines and the concrete interleaving that triggers a race; do not report concurrency issues as abstract possibilities."#;

const STAGE_RESOURCES_INSTRUCTION: &str = r#"# Audit resource lifetime and growth

The Go runtime frees memory, not resources. Audit what the change acquires and what bounds its growth.
- Leaks, especially on error paths: goroutines, watches, informer event handlers that are never removed, timers and tickers, HTTP response bodies that are not closed, files, sockets, gRPC connections and streams, mounts, and temporary files.
- Unbounded growth: a map, slice, cache, or queue keyed by object, by user input, or by time that is not pruned when the object goes away; per-object state that outlives the object; metric label values that are not bounded.
- Cost that grows with the cluster: work per sync that is linear or worse in the number of pods, nodes, or objects; deep copies of large objects on a hot path; a full relist.
- External state that a component owns. For the kubelet: containers, sandboxes, volumes and mounts, cgroups, device and CPU assignments, and checkpoint files. For kube-proxy: iptables, nftables, and IPVS rules and conntrack entries. For controllers: cloud load balancers, routes, and volumes. Check that it is cleaned up when the object goes away, after a restart of the component, and when creation fails halfway.
- API objects as resources: an object that a controller creates needs an owner reference or an explicit cleanup path, a finalizer must be removed, and a lease must be released.
- Restart safety: state held only in memory must be rebuilt from the API server or from the node after a restart. Check what is lost.
- Do not stop after finding several bugs in one function or hunk; systematically audit every modified acquisition and release across the entire diff before concluding.
If it is not provable from the code that every acquired resource is released and every collection is bounded, raise a concern with the concrete leak or growth path."#;

const STAGE_SECURITY_INSTRUCTION: &str = r#"# Audit security boundaries and untrusted input

Kubernetes clusters are shared by tenants with different rights. Flag all potential security issues immediately.
- Who can reach it: name the identity that can trigger the modified code: an anonymous client, an authenticated user with rights in one namespace, the service account of a workload, a node, or a cluster administrator. A defect that only an administrator can trigger against their own cluster is not a security defect.
- Authorization and admission: a check that a request can skip through a subresource, another verb, another version or group of the same resource, or an update path that does not repeat what create checks; a default RBAC role that gains rights; the order of admission plugins.
- Node boundary: a kubelet may act only on its own Node and on the pods bound to it. Flag a change to the node authorizer, to `NodeRestriction`, or to an API that kubelets call, that lets one node reach the objects or secrets of another.
- Fields that users write and privileged components use: object fields that become host paths, mount options, command arguments, environment, URLs, addresses, or file names in the kubelet, kube-proxy, a controller, or the API server. Check the validation in the API server and the handling at the point of use.
- Paths on a node: a container controls the content of its volumes, including symbolic links. Check for traversal with `..`, for a link followed out of the volume, and for a gap between a check and the mount or open that relies on it.
- Requests made by the server: a URL or address taken from an object, to which the API server or a controller then connects, such as a webhook, an aggregated API, or a proxy subresource.
- Secrets: secret data, tokens, or kubeconfig content in logs, events, error messages, metrics, or status; a comparison of secret values that is not constant time; a credential that lives longer or reaches further than needed.
- References across namespaces: a user in one namespace who names an object in another namespace that they may not read.
- Exhaustion by an ordinary user: request bodies, lists, and fields with no size limit, regular expressions or CEL expressions that users supply, and unbounded watches or connections.
- Identity: impersonation, token review, and certificate signing paths that accept an identity without proving it.
- Do not stop after finding several bugs in one function or hunk; systematically audit every path that handles a request or a user-written field across the entire diff before concluding."#;

const STAGE_TESTS_INSTRUCTION: &str = r#"# Audit tests

Audit the tests that the change adds or modifies, and whether a change of behavior has a test at all.
- Detection: would the test fail without the code change? Flag a test that asserts nothing about the changed behavior, that only checks a fake it configured itself, or whose expected values are computed by the code under test.
- Coverage of the change: a bug fix has a test that reproduces the bug; a feature gate has tests for the enabled and the disabled state; a validation change tests create and update, including an update of an object that is already invalid; a new API field is covered by round-trip and defaulting tests.
- Timing: `time.Sleep` to wait for asynchronous work, a timeout too short for a loaded CI machine, and polling with no timeout.
- Isolation: package or global state that is changed and not restored, such as feature gates, environment variables, and global registries; fixed ports and fixed file paths; a test that depends on the order tests run in; `t.Parallel()` together with shared state.
- Determinism: an assertion that depends on map iteration order, on the order of concurrent events, on the exact text of an error from another package, or on wall-clock time.
- Test level: a unit test must not need a cluster or the network. An end-to-end test must not depend on specific Events, on one provider, or on being the only test that runs, unless it is marked for that.
- Leaks from tests: goroutines, servers, and informers that a test starts and does not stop.
- Do not stop after finding several problems in one test; systematically audit every added or modified test across the entire diff before concluding.
Report a test finding at `Low` severity, unless the test passes when the code is wrong or leaks state into other tests. Name the code defect that the test would miss, or the condition under which it fails at random."#;

const STAGE_VERIFICATION_INSTRUCTION: &str = r#"# Verification and severity estimation

You are the lead reviewer consolidating `concerns` and `dismissed_concerns` generated by parallel Kubernetes review stages.
Your task is to (1) deduplicate overlapping items across both lists while preserving every distinct failure mechanism and location, and (2) classify every consolidated item into one of two categories:
- **Category 1: Well-Justified** — either a **1a: Well-Justified Concern** (`findings` array) or a **1b: Well-Justified Dismissal** (`dismissed_concerns` array).
- **Category 2: Inconclusive** (`inconclusive_concerns` array) — routed to parallel per-finding `post-verification` stages for deep tool-assisted code inspection.

### Step 1: Deduplication and Boundary Preservation
1. Group `concerns` and `dismissed_concerns` that refer to the same underlying root cause AND the same function/lifecycle phase.
2. Do NOT merge a create-path bug with an update-path or delete-path bug in a separate function; if an input concern combines bugs across separate functions, split them into separate items. Similarly, do NOT merge distinct races or bugs in separate functions/handlers, or bugs on different resources/fields (e.g., a leaked goroutine vs. a dropped queue item) even within the same function—either keep them as separate items or explicitly name ALL distinct resources, functions, and failure mechanisms in the item's title/description and explanation fields (`problem` and `severity_explanation` for `findings`, or `description` and `concern_arguments` for `inconclusive_concerns`).
3. SPECIFICITY REQUIREMENT: When merging overlapping items, preserve and consolidate the most specific details: exact function names, file paths, line numbers when known, and ALL distinct triggering conditions, callers, consequences, and racing goroutines/handlers mentioned across the merged items. When a missing cleanup, unchecked error, or incomplete error-path unwind causes both an immediate failure and a downstream resource/state impact (e.g., a leaked watch, an object left half-updated, or a corrupted cache entry), explicitly state BOTH consequences in the item's explanation (`severity_explanation` for `findings`, or `concern_arguments` for `inconclusive_concerns`). Preserve and merge the `locations` arrays. Do not invent line numbers; use `null` when unknown.
4. Set `"preexisting": false` whenever the patch introduces the bug or newly exposes, triggers, or makes reachable a latent defect that could not be triggered before the patch (for example, adding a new caller that violates a helper's preconditions or adding an operation before an existing error path that does not clean it up). Set `"preexisting": true` when the problem already existed at `Baseline SHA` (before the commit/series) and was already triggerable before the patch—even if it lives in the same function, caller/callee, or error path modified by the patch without fixing it. Never dismiss a real bug into `dismissed_concerns` or drop it merely because it is pre-existing; validate it in `findings` or route it to `inconclusive_concerns` with `"preexisting": true`. (If a bug was introduced by an earlier commit in the same series after `Baseline SHA`, it is not pre-existing at `Baseline SHA`.)

### Step 2: Classification Rules (1: Well-Justified vs. 2: Inconclusive)
Classify every consolidated item using these strict signal rules. **Repetition is NOT justification: multiple overlapping items are never enough for Category 1 (1a or 1b) unless their reasoning is backed by specific, concrete code proof.**

1. **Category 1a — Well-Justified Concern (`findings` array):**
   - **Mandatory prerequisite & strong signals:** Concrete, self-contained code proof directly visible in the target diff, prefetched context, and cited `locations`, with **no** competing `dismissed_concern`, **no** reliance on unverified assumptions about unseen code (such as callers, callees, or the rest of an unmodified/partially-shown function truncated at the edge of a diff hunk), and **no** potential resolution in subsequent patches of this series (`=== Patch Series Context ===`). Do NOT place suspected build, compilation, lint, or generated-code noise into `findings`; route any such concern to `inconclusive_concerns` so `post-verification` can evaluate and discard it. Multiple deduplicated `concerns` with no attempts to dismiss is a strong signal for 1a **only when grounded in concrete code proof** of the complete function or handler chain.
   - **Action:** Validate it directly and emit it in `findings`. Assign a calibrated `severity` (`Critical`, `High`, `Medium`, or `Low`) strictly following `severity.md`, state all consequences (both immediate and downstream), triggering paths, and reachability at the start of `severity_explanation` (preserving every distinct function, handler, resource, and mechanism), formulate a concise bug title (`problem`) under 80 characters starting with a Kubernetes component or area prefix (e.g. `kubelet:`, `scheduler:`, `apiserver:`, `kube-proxy:`, `client-go:`, `kubectl:`, `api:`), set `"preexisting"`, and include `locations`.

2. **Category 1b — Well-Justified Dismissal (`dismissed_concerns` array):**
   - **Mandatory prerequisite & strong signals:** Concrete disproving `code_snippet` in `locations` (showing the exact local guard, lock, validation rule, feature gate check, cleanup path, or lifecycle invariant that prevents the bug), with **no** competing `concern` and **no** reliance on unverified assumptions about external callers, callees, or the behavior of another component. Multiple overlapping `dismissed_concerns` for the same code is NOT a signal that the dismissal is safe—it indicates multiple analysts independently found the code suspicious; when multiple stages flag and dismiss the same non-trivial mechanism using assumptions about caller behavior, another component, or external state rather than a direct local guard in the same function, classify into `inconclusive_concerns` with `"signal_reason": "speculative_dismissal"`.
   - **Disqualifiers for Category 1b (NEVER classify as 1b):**
     - **Single-caller or happy-path-only proofs:** A dismissal that proves an invariant (such as a non-nil field, a held lock, or a deep copy) in only one caller or normal happy-path mode without verifying ALL callers and entry points in the tree (`git_grep` across event handlers, sync functions, HTTP and admission handlers, registry strategies, command entry points, and interface implementations) is INVALID.
     - **Asymmetry or cleared-state rationalizations:** A dismissal that rationalizes an unpaired lock and unlock, queue `Get` and `Done`, finalizer add and remove, mount and unmount, or start and stop call, an unchecked error return, or clearing/overwriting state before downstream consumers read it as a "harmless no-op" or "intentional behavior" without concrete code proving that exact state or omission is safely handled is INVALID.
     - **Indirect state checks or "harmless side-effect" rationalizations:** A dismissal that rationalizes an unintended state mutation, a changed cache object, a duplicate API write, or an unverified fallback as "harmless" or "benign" without concrete code proof in the downstream consumer is INVALID.
     - **Validation or feature gate cited without proof:** A dismissal that says API validation or defaulting guarantees a value is INVALID when the rule or the field is added by this patch, when objects stored before the rule can break it, or when the object does not come from the API server. A dismissal that says the code is behind a feature gate is INVALID unless it cites the gate's default and lock state at `Target Commit SHA`.
   - **Action:** Place in `dismissed_concerns` (dropped from further verification).
   - **CRITICAL INVARIANT:** Never place any item that was raised as a `concern` into `dismissed_concerns` in this stage. If a raised `concern` is contested by a `dismissed_concern` or appears questionable, it MUST be placed in `inconclusive_concerns` for tool-based `post-verification`.

3. **Category 2 — Inconclusive: Speculative or Contested (`inconclusive_concerns` array — routed to parallel `post-verification`):**
   - **Strong signals:**
     - **Mixed signals (`"mixed_signals"`):** Similar or overlapping `concerns` and `dismissed_concerns` exist for the same root cause, function, or code path.
     - **Speculative or assumption-based dismissal (`"speculative_dismissal"`):** Even when NO stage raised a `concern` (and even when multiple stages emitted overlapping `dismissed_concerns`), inspect every standalone dismissal critically! Multiple overlapping dismissals are NOT enough if they are not justified by specific code. If one or more `dismissed_concerns` identified a plausible bug (such as a missing check on a field that users write, a missing cleanup on an error path, unlocked shared state access, a cache object changed in place, a stricter validation rule applied to stored objects, or a missing nil check on an optional field) and dismissed it using an assumption not proven by the cited `code_snippet` or a vague argument (for example: assuming an unseen caller validates or cleans up on error, or citing only one caller), you MUST classify the item into `inconclusive_concerns` with `"signal_reason": "speculative_dismissal"`.
     - **Speculative or incomplete concern (`"speculative_concern"` or `"insufficient_evidence"`):** One or more overlapping `concerns` whose argument is vague, relies on assumptions not based on specific code in the diff/locations, claims that a guard/cleanup/call is missing from a function whose full body is not shown in the diff (`"signal_reason": "insufficient_evidence"`), mixes a partially inaccurate premise with a potentially real underlying bug in the same code path, or raises a suspected build/linter (`make verify`, `hack/verify-*.sh`, `golangci-lint`), generated-file, documentation snippet (`docs/`, `README*`, comments), or commit-message meta-concern.
     - **Series interaction (`"series_interaction"`):** Any concern that could plausibly be resolved, wired up, or refactored by a subsequent patch listed in `=== Patch Series Context ===`.
   - **Action:** Emit into `inconclusive_concerns` with `"estimated_severity"` (`Critical`, `High`, `Medium`, or `Low`), `"signal_reason"`, `"concern_arguments"`, `"dismissal_arguments"`, a concrete `"verification_question"` specifying what code `post-verification` must inspect with tools, `"preexisting"`, and `"locations"`."#;

const STAGE_POST_VERIFICATION_INSTRUCTION: &str = r#"# Per-finding post-verification and conflict resolution

You are the lead reviewer performing deep, tool-assisted codebase verification of a speculative or contested candidate issue (`inconclusive_concerns`) identified during initial verification.
For each candidate in `inconclusive_concerns`, use the available Git and file tools (`git_read_files`, `git_grep`, `git_diff`, `git_show`, `git_blame`) to answer each candidate's `verification_question` and inspect the actual code in the worktree.
1. Dismiss in `dismissed_concerns` any candidate that alleges a build, compilation, syntax, lint, or stale generated code error of the kind that the compiler or `make verify` (the `hack/verify-*.sh` scripts and `golangci-lint`) reports, and any candidate about the content of a generated or vendored file. A logic error that passes those checks is not such an error and must be verified like any other candidate (carry forward the candidate's target `file`, `function_or_symbol`, and `code_snippet` in `locations`). Dismiss any candidate about a missing or a present `Signed-off-by` trailer, or about a missing area prefix in the subject: Kubernetes requires neither. For a commit message candidate that breaks a rule in the maintainer notes, validate it in `findings` at `Low` severity with `"preexisting": false`.
2. **SYMMETRICAL PROOF BAR & ALL-CALLERS VERIFICATION:** For all other candidates, both `concern_arguments` and `dismissal_arguments` are untrusted hypotheses. Only dismiss a code-behavior candidate in `dismissed_concerns` if concrete, verifiable code in the repository proves the exact failure mechanism, branch, caller, and external interaction cannot occur across ALL callers, entry points, and modes. Citing a single caller does NOT disprove a nil dereference, race, missing validation, or lifecycle leak in a helper function unless `git_grep` across all callers and entry points (event handlers, sync functions, HTTP and admission handlers, registry strategies, command entry points, and interface implementations) proves every caller upholds the invariant. Do NOT give code the benefit of the doubt.
3. **LOCAL BOUNDARY & ASYMMETRY RULE:** Do not discard a defect within the modified code of the patch by assuming that surrounding callers, another component, or a later resync will safely mask or prevent the issue, or by rationalizing an unpaired lock and unlock, queue `Get` and `Done`, finalizer add and remove, or start and stop call, an unchecked error return, or overwritten state as a "harmless no-op" or "intentional behavior", unless you can point to specific code in the repository that concretely proves the failure mode is structurally impossible.
4. **PROMOTING SPECULATIVE DISMISSALS:** When `"signal_reason"` is `"speculative_dismissal"`, a previous analyst spotted the candidate bug described in `concern_arguments` and dismissed it using `dismissal_arguments`. Inspect the actual code with tools: if the dismissal's assumption is false or incomplete (for example: another caller does NOT uphold the invariant; the caller does NOT clean up the resource on an error path; the cited lock does NOT serialize the access against the competing goroutine; the cited validation does NOT run for objects that are already stored; or an unintended state mutation affects downstream consumers), you MUST report the bug as a verified finding in `findings`.
5. **REFINING PARTIALLY INACCURATE PREMISES:** If a candidate concern contains a partially inaccurate premise while also identifying a real bug in the same code path, refine and report the valid underlying bug rather than discarding the entire candidate.
6. **SERIES VALIDATION & PRE-EXISTING BUG RULE:** If other patches in this series are provided in the context, check whether each candidate is resolved, wired up, or refactored in the final state of the series (`Series End Commit`) using tools (`git_read_files` or `git_diff` with `revision` / `target_revision` set to the `Series End Commit`); do not trust promises in commit messages alone. If resolved by the end of the series, dismiss it in `dismissed_concerns` citing the resolving commit. Never dismiss a real bug into `dismissed_concerns` merely because it already existed before this change (`"preexisting": true` findings are routed to the pre-existing bug tracker and are not posted as patch regressions): if a candidate is a real bug that already existed at `Baseline SHA` and is not fixed at `Series End Commit`, report it in `findings` with `"preexisting": true`. Conversely, if a defect was introduced by an earlier commit in the same series (after `Baseline SHA`) and `Target Commit` does not touch, trigger, or worsen it, dismiss it for this commit rather than marking `"preexisting": true`. When referring to other patches within this series in your explanation, DO NOT use ephemeral git hashes; refer to them by their patch subject (e.g. 'commit "kubelet: stop the pod worker on termination"').
7. **DESIGN & DOCUMENTATION RULE:** If a candidate targets illustrative pseudo-code or abbreviated snippets in documentation (`docs/`, `README*` files, code comments), inspect the actual implementation at the series head (`Series End Commit` / `HEAD`). If the actual code properly enforces the invariant, dismiss the documentation concern in `dismissed_concerns` citing the enforcing code.
8. **FEATURE GATE AND VERSION SKEW RULE:** For a candidate that depends on a feature gate, read the gate's default and lock state at `Target Commit SHA` and state them in the explanation. For a candidate that depends on version skew, state the component versions and confirm that the policy supports that pair; dismiss a candidate that needs a component newer than its API server.
9. **SEVERITY CALIBRATION AND COMPLETENESS:** Set `"preexisting": false` whenever the patch introduces the bug or newly exposes, triggers, or makes reachable a latent defect that could not be triggered before the patch; set `"preexisting": true` when the problem already existed at `Baseline SHA` (before the commit/series) and was already triggerable before the patch, even if it lives in the same function, caller/callee, or error path modified by the patch. For each verified issue, assign an accurate severity (`Critical`, `High`, `Medium`, or `Low`) strictly following `severity.md`: reason through all consequences (both immediate failures and downstream state/resource leaks), triggering paths, and reachability, state that reasoning at the start of `severity_explanation`, preserve all distinct function names, file paths, line numbers when known, triggering entry points, and consequences, and formulate a concise bug title (`problem`) under 80 characters starting with a Kubernetes component or area prefix (e.g. `kubelet:`, `scheduler:`, `apiserver:`, `kube-proxy:`, `client-go:`, `kubectl:`, `api:`)."#;

const STAGE_REPORT_INSTRUCTION: &str = r#"# Generate plain-text inline review report

Generate the plain-text inline review report following the exact formatting rules and structure in `report-template.md`.
- Output ONLY a plain bulleted list of ALL findings ordered from highest severity to lowest (`- [CRITICAL] ...`, `- [HIGH] ...`, `- [MEDIUM] ...`, `- [LOW] ...`), or `No issues found.` if there are no findings.
- If any finding has `"preexisting": true`, state explicitly in its explanation that the issue was not introduced by this change.
- Do NOT include `Summary:` or `Findings:` headers.
- Do NOT use backticks, markdown code blocks, or markdown headings. Wrap all lines at 78 characters or fewer."#;

const CONCERN_JSON_SCHEMA_EXAMPLE: &str = r#"Once you have gathered sufficient information, return ONLY a JSON object with 'concerns' and 'dismissed_concerns' arrays.
If you find no concerns and no dismissed concerns, return {"concerns": [], "dismissed_concerns": []}.
Each object in the 'concerns' array MUST use exactly the following keys: "type", "description", "reasoning", "preexisting", "locations".
- "type": A short category string (e.g. "API Compatibility", "Validation / Stored Objects", "Feature Gate", "Version Skew", "Cache Mutation", "Reconciliation", "Locking / Race", "Goroutine Leak", "Resource Leak", "Authorization", "Nil Dereference", "Error Handling", "Test Flake").
Each object in the 'dismissed_concerns' array MUST use exactly the following keys: "type", "description", "reasoning", "locations".
Each location object in "locations" (for both 'concerns' and 'dismissed_concerns') MUST include "file", "function_or_symbol", "line", "code_snippet", "side", and "why_this_location_matters".
- "side": "PARENT" if commenting on deleted or pre-patch code (e.g., '-' lines in the diff or baseline code), or "REVISION" if commenting on newly added or modified code (e.g., '+' lines in the diff or post-patch code). Default to "REVISION" when commenting on unchanged context lines.
In each 'dismissed_concerns' object, "description" is the candidate concern that was investigated and disproved, "reasoning" is the step-by-step explanation of why it is not a bug (citing the exact guard, caller, or invariant), and "locations" MUST cite the concrete disproving code (file, function_or_symbol, line, verbatim code_snippet, side, and why_this_location_matters).
Use the 'dismissed_concerns' array ONLY for candidate concerns that you considered plausible, investigated, and disproved with concrete evidence. This is especially important when you first suspect a concern and then follow the evidence chain proving that it does NOT apply.

PRE-EXISTING BUGS: Do not actively hunt for unrelated bugs outside the reviewed commit and its call paths. However, if while analyzing the commit you spot a concrete bug that already existed in the codebase before this series (`Baseline SHA`) and is not fixed by the commit, you MUST still report it in 'concerns' with `"preexisting": true` (`"preexisting": true` items are routed to the pre-existing bug tracker and are not posted as patch regressions). Never silently drop a real pre-existing bug or place it in 'dismissed_concerns' merely because it was not introduced by this change.

GENERATED AND VENDORED FILES: Do not raise a concern about the content of a file named in `.generated_files`, under `api/openapi-spec/`, or under `vendor/`. Use such a file only as evidence of what a hand-written change does, and place the concern on the hand-written code.

NO DISMISSAL WITHOUT VERIFIED PROOF: To place a candidate issue in 'dismissed_concerns' (or to discard a suspected issue), you MUST find concrete proof in the code ('file', 'function_or_symbol', 'line', and verbatim 'code_snippet' in 'locations') that explicitly invalidates the concern's reasoning. If the disproving code lives outside the diff (for example, in a caller, callee, helper, validation function, or feature gate declaration), you MUST verify that code first using tools ('git_read_files' or 'git_grep') and quote the verified disproving snippet in 'locations'. If you cannot find definitive code proof that the candidate issue is impossible, you MUST report it in 'concerns' (NOT 'dismissed_concerns') and make the condition explicit: if X is possible, then problem Y can occur.
- Citing a single caller does NOT disprove a nil dereference, race, missing validation, or lifecycle leak in a helper function. A caller-based dismissal is valid ONLY if every caller in the tree ('git_grep' across all callers and entry points, including event handlers, sync functions, HTTP and admission handlers, registry strategies, command entry points, and interface implementations) is verified to uphold the invariant; otherwise report it in 'concerns'.
- Never dismiss an unpaired lifecycle/state transition (e.g., a lock that is not released on an error path, a missing queue Done() call, a finalizer that is never removed, or clearing/overwriting state before downstream consumers read it) by rationalizing that the side effect is a "harmless no-op" or "rare edge case".

SPECIFICITY REQUIREMENT: When reporting a concern or dismissed_concern, cite exact function name(s), file path(s), and line number(s) when known. Do not invent line numbers; use null when exact values are unknown.

CRITICAL REVIEW DIRECTIVE: Do NOT dismiss concerns just because you assume the surrounding framework, the validation in the API server, or callers handle it perfectly. Do not be overly charitable to the code. Assume that an informer cache is stale, that any API call can fail or conflict, that the process can crash between any two writes, that goroutines interleave at every point outside a lock, and that users control every field they are allowed to write.

Example Output:
```json
{
  "concerns": [
    {
      "type": "Cache Mutation",
      "description": "Object from the lister is modified without a deep copy",
      "reasoning": "1. syncFoo() gets foo from fooLister.Foos(ns).Get(name), which returns the object held by the shared informer cache.\n2. Line 212 sets foo.Status.Ready on that object before calling UpdateStatus, so the cache and every other reader see the change even if the update fails.",
      "preexisting": false,
      "locations": [
        {
          "file": "pkg/controller/foo/foo_controller.go",
          "function_or_symbol": "syncFoo",
          "line": 212,
          "code_snippet": "foo.Status.Ready = true",
          "side": "REVISION",
          "why_this_location_matters": "The object is shared with the informer cache, so the write corrupts it for all readers."
        }
      ]
    }
  ],
  "dismissed_concerns": [
    {
      "type": "Error Handling",
      "description": "Potential dropped queue item when the status update fails",
      "reasoning": "Verified that processNextWorkItem() calls queue.AddRateLimited(key) on line 180 for every error that syncFoo() returns, so the item is retried with backoff.",
      "locations": [
        {
          "file": "pkg/controller/foo/foo_controller.go",
          "function_or_symbol": "processNextWorkItem",
          "line": 180,
          "code_snippet": "c.queue.AddRateLimited(key)",
          "side": "REVISION",
          "why_this_location_matters": "Proves the failed item is added to the queue again."
        }
      ]
    }
  ]
}
```"#;

// ---------------------------------------------------------------------------
// Stage Table Definitions
// ---------------------------------------------------------------------------

pub static ANALYSIS_STAGES: &[AnalysisStage] = &[
    AnalysisStage {
        name: "goal",
        short: "Goal Analysis",
        instruction: STAGE_GOAL_INSTRUCTION,
        guides: &[],
        uses_commit_log: true,
        optional: false,
        wants_series_context: true,
    },
    AnalysisStage {
        name: "implementation",
        short: "Implementation",
        instruction: STAGE_IMPLEMENTATION_INSTRUCTION,
        guides: &[],
        uses_commit_log: true,
        optional: false,
        wants_series_context: true,
    },
    AnalysisStage {
        name: "execution-flow",
        short: "Execution Flow",
        instruction: STAGE_EXECUTION_FLOW_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: false,
        wants_series_context: false,
    },
    AnalysisStage {
        name: "api-compat",
        short: "API & Compat",
        instruction: STAGE_API_COMPAT_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: true,
        wants_series_context: true,
    },
    AnalysisStage {
        name: "reconciliation",
        short: "Reconciliation",
        instruction: STAGE_RECONCILIATION_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: true,
        wants_series_context: false,
    },
    AnalysisStage {
        name: "concurrency",
        short: "Concurrency",
        instruction: STAGE_CONCURRENCY_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: true,
        wants_series_context: false,
    },
    AnalysisStage {
        name: "resources",
        short: "Resource Mgmt",
        instruction: STAGE_RESOURCES_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: true,
        wants_series_context: false,
    },
    AnalysisStage {
        name: "security",
        short: "Security Audit",
        instruction: STAGE_SECURITY_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: true,
        wants_series_context: false,
    },
    AnalysisStage {
        name: "tests",
        short: "Test Audit",
        instruction: STAGE_TESTS_INSTRUCTION,
        guides: &[],
        uses_commit_log: false,
        optional: true,
        wants_series_context: true,
    },
];

pub static VERIFICATION: ConsolidationStage = ConsolidationStage {
    name: "verification",
    short: "Severity Estimation",
    wants_series_context: true,
};

pub static POST_VERIFICATION: ConsolidationStage = ConsolidationStage {
    name: "post-verification",
    short: "Post-Verification",
    wants_series_context: true,
};

pub static REPORT: ConsolidationStage = ConsolidationStage {
    name: "report",
    short: "Report Generation",
    wants_series_context: false,
};

pub static CONSOLIDATION_STAGES: &[&ConsolidationStage] =
    &[&VERIFICATION, &POST_VERIFICATION, &REPORT];

fn series_context_placeholder(wants: bool) -> &'static str {
    if wants {
        SERIES_CONTEXT_PLACEHOLDER
    } else {
        ""
    }
}

fn with_series_context(
    template: PromptTemplate<KubernetesPatchReviewState>,
    wants: bool,
) -> PromptTemplate<KubernetesPatchReviewState> {
    if !wants {
        return template;
    }
    template.with_var(
        "follow_up_series_section",
        |s: &KubernetesPatchReviewState| {
            s.follow_up_series_context
                .as_ref()
                .map(|ctx| format!("\n\n{}", ctx))
                .unwrap_or_default()
        },
    )
}

pub fn analysis_stage_by_name(name: &str) -> Option<&'static AnalysisStage> {
    let normalized = normalize_stage_name(name);
    ANALYSIS_STAGES.iter().find(|s| s.name == normalized)
}

pub fn consolidation_stage_by_name(name: &str) -> Option<&'static ConsolidationStage> {
    let normalized = normalize_stage_name(name);
    if POST_VERIFICATION_STAGE_NAMES.contains(&normalized.as_str()) {
        return Some(&POST_VERIFICATION);
    }
    CONSOLIDATION_STAGES
        .iter()
        .copied()
        .find(|s| s.name == normalized)
}

pub fn stage_short_label(name: &str) -> Option<&'static str> {
    if let Some(def) = analysis_stage_by_name(name) {
        return Some(def.short);
    }
    if normalize_stage_name(name) == COMBINED_ANALYSIS_STAGE_NAME {
        return Some(COMBINED_ANALYSIS_SHORT_LABEL);
    }
    consolidation_stage_by_name(name).map(|s| s.short)
}

// ---------------------------------------------------------------------------
// Stage Builders
// ---------------------------------------------------------------------------

pub fn validate_kubernetes_planning_output_with_effort(
    out: &PlanningOutput,
    _state: &KubernetesPatchReviewState,
) -> Result<(), String> {
    validate_planning_stage_efforts(analysis_stage_by_name, out)
}

pub fn planning_stage_with_effort(
    temperature: f32,
    effort: Option<crate::ai::EffortMode>,
) -> Stage<KubernetesPatchReviewState, PlanningOutput> {
    let dynamic_effort = effort == Some(crate::ai::EffortMode::Dynamic);
    let optional_stages: Vec<&'static str> = ANALYSIS_STAGES
        .iter()
        .filter(|s| s.optional)
        .map(|s| s.name)
        .collect();
    let optional_list = optional_stages.join(", ");

    let (sys_prompt, output_format) = if dynamic_effort {
        let stage_efforts_schema = build_stage_efforts_schema(ANALYSIS_STAGES);
        (
            format!(
                "You are an AI assistant planning a Kubernetes patch review.\nThe core stages (goal, implementation, execution-flow) always run.\n1. Select which optional specialized stages should also run based on the patch contents.\nAvailable optional stages: [{optional_list}]\nInclude an optional stage when the patch touches or interacts with that domain; omit stages that are clearly irrelevant to the modified code. Generated and vendored files alone do not make a stage relevant.\n\n2. Select the reasoning effort (\"medium\" or \"high\") in stage_efforts for each stage that will run (goal, implementation, execution-flow, plus each stage selected in relevant_stages):\n- Use \"high\" for stages whose domain is central to the patch or where API compatibility, complex control flow, subtle error paths, concurrency, or handling of fields that users write must be verified.\n- Use \"medium\" for stages performing a routine check on a straightforward aspect of the patch.\n\nRespond with ONLY a JSON object. Example:\n```json\n{{\"relevant_stages\": [\"api-compat\", \"concurrency\"], \"stage_efforts\": {{\"goal\": \"medium\", \"implementation\": \"high\", \"execution-flow\": \"high\", \"api-compat\": \"high\", \"concurrency\": \"medium\"}}}}\n```"
            ),
            OutputFormat::json_with_schema(json!({
                "type": "object",
                "properties": {
                    "relevant_stages": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "stage_efforts": stage_efforts_schema
                },
                "required": ["relevant_stages", "stage_efforts"]
            }))
            .with_validator(validate_kubernetes_planning_output_with_effort)
            .with_feedback_formatter(format_planning_feedback),
        )
    } else {
        (
            format!(
                "You are an AI assistant planning a Kubernetes patch review.\nThe core stages (goal, implementation, execution-flow) always run.\nSelect which optional specialized stages should also run based on the patch contents.\nAvailable optional stages: [{optional_list}]\n\nCRITICAL BIAS RULE: Err on the side of inclusion. Include any stage whose domain could plausibly be affected by the hand-written code in the patch. Generated and vendored files alone do not make a stage relevant.\n\nRespond with ONLY a JSON object listing the relevant optional stages. Example:\n```json\n{{\"relevant_stages\": [\"api-compat\", \"concurrency\"]}}\n```"
            ),
            OutputFormat::json_with_schema(json!({
                "type": "object",
                "properties": {
                    "relevant_stages": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["relevant_stages"]
            })),
        )
    };

    Stage::builder("planning")
        .system_prompt(PromptTemplate::<KubernetesPatchReviewState>::new(
            sys_prompt,
        ))
        .user_prompt(
            PromptTemplate::<KubernetesPatchReviewState>::new(
                "<patch>\n{{target_commit_diff}}\n</patch>",
            )
            .with_var("target_commit_diff", |s: &KubernetesPatchReviewState| {
                s.target_commit_diff.clone()
            }),
        )
        .output_format(output_format)
        .policy(StagePolicy {
            tools: ToolScope::None,
            max_turns: 1,
            temperature,
            reasoning_effort: effort.map(crate::ai::EffortMode::lightweight_stage_effort),
            ..Default::default()
        })
        .skip_if(|s| s.cheap_mode || s.manual_stages.is_some())
        .reduce(move |state, out: PlanningOutput| {
            apply_planning_stage_output(
                ANALYSIS_STAGES,
                analysis_stage_by_name,
                dynamic_effort,
                out,
                &mut state.planned_stages,
                &mut state.planned_efforts,
            );
        })
        .build()
}

fn analysis_stage(
    def: &'static AnalysisStage,
    max_turns: usize,
    temperature: f32,
    reasoning_effort: Option<crate::ai::ReasoningEffort>,
) -> Box<dyn ExecutableStage<KubernetesPatchReviewState>> {
    let series_context = series_context_placeholder(def.wants_series_context);
    let mut user_template = PromptTemplate::<KubernetesPatchReviewState>::new(format!(
        "{}{}\n\n{}",
        def.instruction, series_context, CONCERN_JSON_SCHEMA_EXAMPLE
    ));

    for guide in def.guides {
        user_template = user_template.include_file(*guide);
    }

    let user_template = with_series_context(user_template, def.wants_series_context);

    Box::new(
        Stage::builder(def.name)
            .required()
            .system_prompt(kubernetes_system_prompt(def.uses_commit_log))
            .user_prompt(user_template)
            .output_format(
                OutputFormat::json()
                    .with_validator(validate_concerns_output)
                    .with_feedback_formatter(format_concerns_feedback),
            )
            .policy(StagePolicy {
                tools: ToolScope::All,
                max_turns,
                temperature,
                reasoning_effort,
                ..Default::default()
            })
            .reduce(
                move |state: &mut KubernetesPatchReviewState, out: StageConcernsOutput| {
                    let prompts = collect_stage_prompts(&state.selected_guides, def.guides);
                    append_stage_items_with_prompts(
                        &mut state.all_concerns,
                        &out.concerns,
                        def.name,
                        "General",
                        &prompts,
                    );
                    append_stage_dismissed_concerns_with_prompts(
                        &mut state.all_dismissed_concerns,
                        &out.dismissed_concerns,
                        def.name,
                        &prompts,
                    );
                },
            )
            .build(),
    )
}

pub fn resolve_analysis_stages_with_options(
    state: &KubernetesPatchReviewState,
    max_turns: usize,
    temperature: f32,
) -> Vec<Box<dyn ExecutableStage<KubernetesPatchReviewState>>> {
    let selected_stages: Vec<String> = if let Some(ref manual) = state.manual_stages {
        manual.clone()
    } else if !state.planned_stages.is_empty() {
        state.planned_stages.clone()
    } else {
        ANALYSIS_STAGES.iter().map(|d| d.name.to_string()).collect()
    };

    let mut use_combined = state.cheap_mode;
    let mut selected_defs = Vec::new();
    for name in selected_stages {
        if normalize_stage_name(&name) == COMBINED_ANALYSIS_STAGE_NAME {
            use_combined = true;
            for def in ANALYSIS_STAGES {
                if !selected_defs
                    .iter()
                    .any(|d: &&AnalysisStage| d.name == def.name)
                {
                    selected_defs.push(def);
                }
            }
            continue;
        }
        match analysis_stage_by_name(&name) {
            Some(def) => {
                if !selected_defs
                    .iter()
                    .any(|d: &&AnalysisStage| d.name == def.name)
                {
                    selected_defs.push(def);
                }
            }
            None => tracing::warn!("Ignoring unknown Kubernetes review stage {:?}", name),
        }
    }

    let mut stages = Vec::new();
    if use_combined {
        if !selected_defs.is_empty() {
            let effort = resolve_combined_stage_effort(state);
            stages.push(build_combined_analysis_stage(
                selected_defs,
                kubernetes_system_prompt,
                CONCERN_JSON_SCHEMA_EXAMPLE,
                false,
                validate_concerns_output,
                format_concerns_feedback,
                max_turns,
                temperature,
                effort,
            ));
        }
    } else {
        for def in selected_defs {
            let effort = resolve_stage_effort(state, def.name);
            stages.push(analysis_stage(def, max_turns, temperature, effort));
        }
    }
    stages
}

pub fn verification_stage(
    max_turns: usize,
    temperature: f32,
) -> Stage<KubernetesPatchReviewState, VerificationOutput> {
    verification_stage_with_effort(max_turns, temperature, None)
}

pub fn verification_stage_with_effort(
    max_turns: usize,
    temperature: f32,
    effort: Option<crate::ai::EffortMode>,
) -> Stage<KubernetesPatchReviewState, VerificationOutput> {
    let series_context = series_context_placeholder(VERIFICATION.wants_series_context);
    let user_template = with_series_context(
        PromptTemplate::<KubernetesPatchReviewState>::new(format!(
            r#"{STAGE_VERIFICATION_INSTRUCTION}

<false_positive_guide>
@include("false-positive-guide.md")
</false_positive_guide>

<severity_guidelines>
@include("severity.md")
</severity_guidelines>@includes

CRITICAL REVIEW DIRECTIVE: To dismiss a concern as a false positive, you must have concrete evidence in the code that proves the concern is invalid. Never drop a raised concern into 'dismissed_concerns' in this stage: every consolidated concern must be placed either in 'findings' (if well-justified with concrete code proof and no competing dismissal) or in 'inconclusive_concerns' (if contested, speculative, or requiring tool verification). Also inspect every standalone dismissed_concern: if it dismissed a plausible bug using an unverified assumption, promote it into 'inconclusive_concerns' with '"signal_reason": "speculative_dismissal"'.{series_context}

Aggregated Concerns:
{{{{aggregated_concerns}}}}

Aggregated Dismissed Concerns:
{{{{aggregated_dismissed_concerns}}}}

Return ONLY a JSON object with 'findings', 'inconclusive_concerns', and 'dismissed_concerns' arrays.
- LINEAGE REQUIREMENT ('source_ids'): Every item in Aggregated Concerns has an "id" ("C1", "C2", ...) and every item in Aggregated Dismissed Concerns has an "id" ("D1", "D2", ...). Every object in 'findings', 'inconclusive_concerns', and 'dismissed_concerns' MUST include a non-empty "source_ids" array of strings listing the exact input "C*" and/or "D*" IDs consolidated into that output item. Every input "C*" and "D*" ID MUST appear in at least one output item's "source_ids". Category 1a 'findings' may ONLY reference "C*" IDs (any contested "C*" + "D*" or promoted "D*" must be routed to 'inconclusive_concerns'), and Category 1b 'dismissed_concerns' may ONLY reference "D*" IDs in "source_ids" (never "C*" IDs).
- Each object in 'findings' (Category 1a: Well-Justified Concerns) MUST use the keys: "source_ids" (non-empty array of "C*" IDs only), "problem" (a short naming string under 80 characters starting with a Kubernetes component or area prefix like 'kubelet:', 'scheduler:', 'apiserver:', 'kube-proxy:', 'client-go:', 'kubectl:', 'api:', NEVER using backquotes), "severity" ("Low", "Medium", "High", "Critical", or "Unknown"), "severity_explanation" (detailed reasoning and proof), "preexisting" (boolean), and "locations" (array of objects with file, function_or_symbol, line, code_snippet, side, and why_this_location_matters; side MUST be "PARENT" for deleted/pre-patch code or "REVISION" for added/modified/context code), and may include "stages" (array of stage names) and "prompts" (array of prompt files).
- Each object in 'inconclusive_concerns' (Category 2: Inconclusive) MUST use the keys: "source_ids" (non-empty array of input IDs), "type", "description", "estimated_severity" ("Low", "Medium", "High", "Critical", or "Unknown"), "signal_reason" ("mixed_signals", "speculative_concern", "speculative_dismissal", "series_interaction", or "other"), "concern_arguments" (consolidated arguments for why the bug can occur), "dismissal_arguments" (consolidated arguments/snippets from any competing or standalone dismissal, or "" if none), "verification_question" (the specific code question post-verification must answer with tools), "preexisting" (boolean), and "locations" (array of objects with file, function_or_symbol, line, code_snippet, side, and why_this_location_matters; side MUST be "PARENT" or "REVISION"), and may include "stages" and "prompts".
- Each object in 'dismissed_concerns' (Category 1b: Well-Justified Dismissals) MUST use the keys: "source_ids" (non-empty array of "D*" IDs only), "type", "description", "reasoning", and "locations" (array of objects with file, function_or_symbol, line, code_snippet, side, and why_this_location_matters; side MUST be "PARENT" or "REVISION"), and may include "stages" and "prompts"."#
        ))
        .include_file("false-positive-guide.md")
        .include_file("severity.md")
        .include_files_from_state(|s: &KubernetesPatchReviewState| {
            extra_prompt_paths_for_state(s, analysis_stage_by_name)
        }),
        VERIFICATION.wants_series_context,
    )
    .with_var("aggregated_concerns", |s: &KubernetesPatchReviewState| {
        serde_json::to_string_pretty(&s.all_concerns).unwrap_or_default()
    })
    .with_var(
        "aggregated_dismissed_concerns",
        |s: &KubernetesPatchReviewState| {
            serde_json::to_string_pretty(&s.all_dismissed_concerns).unwrap_or_default()
        },
    );

    Stage::builder(VERIFICATION.name)
        .system_prompt(kubernetes_system_prompt(true))
        .user_prompt(user_template)
        .output_format(
            OutputFormat::json()
                .with_validator(validate_verification_stage_output)
                .with_feedback_formatter(format_verification_stage_feedback),
        )
        .policy(StagePolicy {
            tools: ToolScope::All,
            max_turns,
            temperature,
            reasoning_effort: effort.map(crate::ai::EffortMode::verification_stage_effort),
            ..Default::default()
        })
        .skip_if(|s| s.all_concerns.is_empty() && s.all_dismissed_concerns.is_empty())
        .reduce(|state, out: VerificationOutput| {
            apply_verification_stage_output(state, out, analysis_stage_by_name);
        })
        .build()
}

pub fn post_verification_stage(
    stage_name: &'static str,
    batch: Vec<Value>,
    max_turns: usize,
    temperature: f32,
) -> Stage<KubernetesPatchReviewState, PostVerificationOutput> {
    post_verification_stage_with_effort(stage_name, batch, max_turns, temperature, None)
}

pub fn post_verification_stage_with_effort(
    stage_name: &'static str,
    batch: Vec<Value>,
    max_turns: usize,
    temperature: f32,
    effort: Option<crate::ai::EffortMode>,
) -> Stage<KubernetesPatchReviewState, PostVerificationOutput> {
    let candidate_json = serde_json::to_string_pretty(&batch).unwrap_or_default();
    let batch_for_prompts = batch.clone();
    let batch_for_validate = batch.clone();
    let batch_for_reduce = batch;
    let series_context = series_context_placeholder(POST_VERIFICATION.wants_series_context);
    let user_template = with_series_context(
        PromptTemplate::<KubernetesPatchReviewState>::new(format!(
            r#"{STAGE_POST_VERIFICATION_INSTRUCTION}

<false_positive_guide>
@include("false-positive-guide.md")
</false_positive_guide>

<severity_guidelines>
@include("severity.md")
</severity_guidelines>@includes

CRITICAL REVIEW DIRECTIVE: To dismiss a code-behavior candidate issue as a false positive, you must find concrete evidence in the code that proves the issue is invalid and quote that disproving code in `dismissed_concerns[].locations` (for policy-excluded build/linter, generated-file, or out-of-scope meta-concerns under rule 1, carry forward the candidate's target location or commit message snippet in `locations`). If you cannot find concrete proof of safety, you must validate and report the finding in `findings`.{series_context}

Inconclusive Concern(s) to Verify:
{{{{candidate_inconclusive_concerns}}}}

Return ONLY a JSON object with 'findings' and 'dismissed_concerns' arrays. Every candidate in this batch MUST be accounted for in either 'findings' (if validated) or 'dismissed_concerns' (if disproved by concrete code or excluded by rule 1; never return both empty arrays).
- LINEAGE REQUIREMENT ('source_ids'): Every inconclusive concern in this batch has an "id" ("I1", "I2", ...). Every object in 'findings' and 'dismissed_concerns' MUST include a non-empty "source_ids" array of strings listing the exact candidate "I*" ID(s) from this batch that it resolves, and every candidate "I*" ID in this batch MUST be accounted for in at least one output item's "source_ids".
- Each object in 'findings' MUST use: "source_ids" (non-empty array of batch "I*" IDs), "problem" (a short naming string under 80 characters starting with a Kubernetes component or area prefix like 'kubelet:', 'scheduler:', 'apiserver:', 'kube-proxy:', 'client-go:', 'kubectl:', 'api:', NEVER using backquotes), "severity" (Low, Medium, High, Critical, or Unknown), "severity_explanation" (detailed reasoning and proof), "preexisting" (boolean), "locations" (array of objects with file, function_or_symbol, line, code_snippet, side, and why_this_location_matters; side MUST be "PARENT" for deleted/pre-patch code or "REVISION" for added/modified/context code).
- Each object in 'dismissed_concerns' MUST use: "source_ids" (non-empty array of batch "I*" IDs), "description" (the candidate issue that was disproved), "reasoning" (step-by-step explanation of how the inspected code or policy rule disproves the candidate), and "locations" (a non-empty array of objects with file, function_or_symbol, line, code_snippet, side, and why_this_location_matters; side MUST be "PARENT" or "REVISION", quoting the verbatim disproving code or target snippet)."#
        ))
        .include_file("false-positive-guide.md")
        .include_file("severity.md")
        .include_files_from_state(move |s: &KubernetesPatchReviewState| {
            extra_prompt_paths_for_items(
                &s.selected_guides,
                &batch_for_prompts,
                analysis_stage_by_name,
            )
        }),
        POST_VERIFICATION.wants_series_context,
    )
    .with_var(
        "candidate_inconclusive_concerns",
        move |_: &KubernetesPatchReviewState| candidate_json.clone(),
    );

    Stage::builder(stage_name)
        .required()
        .system_prompt(kubernetes_system_prompt(true))
        .user_prompt(user_template)
        .output_format(
            OutputFormat::json()
                .with_validator(
                    move |out: &PostVerificationOutput, _: &KubernetesPatchReviewState| {
                        validate_post_verification_batch_items(out, &batch_for_validate)
                    },
                )
                .with_feedback_formatter(format_post_verification_feedback),
        )
        .policy(StagePolicy {
            tools: ToolScope::All,
            max_turns,
            temperature,
            reasoning_effort: effort.map(crate::ai::EffortMode::verification_stage_effort),
            ..Default::default()
        })
        .reduce(move |state, out: PostVerificationOutput| {
            apply_post_verification_stage_output(
                state,
                stage_name,
                &batch_for_reduce,
                out,
                analysis_stage_by_name,
            );
        })
        .build()
}

pub fn resolve_post_verification_stages_with_options(
    state: &KubernetesPatchReviewState,
    max_turns: usize,
    temperature: f32,
) -> Vec<Box<dyn ExecutableStage<KubernetesPatchReviewState>>> {
    let effort = state.effort;
    let batches = batch_inconclusive_concerns_by_severity(&state.inconclusive_concerns);
    batches
        .into_iter()
        .enumerate()
        .map(|(idx, batch)| {
            let stage_name = POST_VERIFICATION_STAGE_NAMES[idx];
            Box::new(post_verification_stage_with_effort(
                stage_name,
                batch,
                max_turns,
                temperature,
                effort,
            )) as Box<dyn ExecutableStage<KubernetesPatchReviewState>>
        })
        .collect()
}

pub fn report_stage(
    max_turns: usize,
    temperature: f32,
) -> Stage<KubernetesPatchReviewState, String> {
    report_stage_with_effort(max_turns, temperature, None)
}

pub fn report_stage_with_effort(
    max_turns: usize,
    temperature: f32,
    effort: Option<crate::ai::EffortMode>,
) -> Stage<KubernetesPatchReviewState, String> {
    Stage::builder(REPORT.name)
        .system_prompt(kubernetes_system_prompt(true))
        .user_prompt(
            PromptTemplate::<KubernetesPatchReviewState>::new(format!(
                r#"{STAGE_REPORT_INSTRUCTION}

<report_template>
@include("report-template.md")
</report_template>

Findings:
{{{{findings}}}}

Return strictly plain text output (no markdown, no backticks, wrapped at 78 characters), not JSON."#
            ))
            .include_file("report-template.md")
            .with_var("findings", |s: &KubernetesPatchReviewState| {
                serde_json::to_string_pretty(&s.findings).unwrap_or_default()
            }),
        )
        .output_format(OutputFormat::text_with_validator(
            validate_qemu_report_format,
            format_qemu_report_feedback,
        ))
        .policy(StagePolicy {
            tools: ToolScope::All,
            max_turns,
            temperature,
            reasoning_effort: effort.map(crate::ai::EffortMode::report_stage_effort),
            recitation_policy: RecitationPolicy::FallbackToFreeForm {
                reminder: "Do not quote large blocks of code verbatim. Summarize concisely."
                    .to_string(),
            },
            ..Default::default()
        })
        .skip_if(|s| s.skip_report || s.findings.is_empty())
        .reduce(|state, out: String| {
            state.review_inline = format_qemu_inline_findings(&out);
        })
        .build()
}

// ---------------------------------------------------------------------------
// Complete Kubernetes Review Workflow Graph
// ---------------------------------------------------------------------------

pub fn build_kubernetes_patch_review_workflow() -> Workflow<KubernetesPatchReviewState> {
    build_kubernetes_patch_review_workflow_with_options(20, 0.0)
}

/// Builds the Kubernetes patch review workflow.
///
/// Like [`crate::workflows::zephyr_patch_review::build_zephyr_patch_review_workflow_with_effort`],
/// the workflow early-exits when the analysis fan-out produced no concerns or
/// dismissed concerns, and again when verification validated no findings, so
/// a clean patch costs only the planning and core analysis stages.
pub fn build_kubernetes_patch_review_workflow_with_options(
    max_turns: usize,
    temperature: f32,
) -> Workflow<KubernetesPatchReviewState> {
    build_kubernetes_patch_review_workflow_with_effort(max_turns, temperature, None)
}

pub fn build_kubernetes_patch_review_workflow_with_effort(
    max_turns: usize,
    temperature: f32,
    effort: Option<crate::ai::EffortMode>,
) -> Workflow<KubernetesPatchReviewState> {
    let stage_max_turns = max_turns.min(20);
    Workflow::builder("kubernetes_patch_review")
        .dynamic_parallel(
            planning_stage_with_effort(temperature, effort),
            move |state| resolve_analysis_stages_with_options(state, stage_max_turns, temperature),
            ParallelPolicy::BestEffort,
        )
        .early_exit_if(
            |s| s.all_concerns.is_empty() && s.all_dismissed_concerns.is_empty(),
            "No concerns or dismissed concerns raised in initial analysis stages",
        )
        .dynamic_parallel(
            verification_stage_with_effort(stage_max_turns, temperature, effort),
            move |state| {
                resolve_post_verification_stages_with_options(state, stage_max_turns, temperature)
            },
            ParallelPolicy::BestEffort,
        )
        .early_exit_if(
            |s| s.findings.is_empty(),
            "No findings validated in verification stages",
        )
        .stage(report_stage_with_effort(
            stage_max_turns,
            temperature,
            effort,
        ))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPTS_DIR: &str = "projects/kubernetes/prompts";

    fn selected_notes(files: &[&str], diff: &str) -> Vec<String> {
        let prompts_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(PROMPTS_DIR);
        let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
        let notes = crate::notes::maintainer_notes::load_maintainer_notes(&prompts_dir);
        let idents = crate::notes::maintainer_notes::diff_identifiers_all(diff);
        notes
            .iter()
            .filter(|n| n.applies(&files, &idents))
            .map(|n| n.name.clone())
            .collect()
    }

    fn assert_notes(selected: &[String], present: &[&str], absent: &[&str]) {
        for note in present {
            assert!(
                selected.iter().any(|n| n == note),
                "{note} must load: {selected:?}"
            );
        }
        for note in absent {
            assert!(
                !selected.iter().any(|n| n == note),
                "{note} must not load: {selected:?}"
            );
        }
    }

    #[test]
    fn test_kubernetes_analysis_stages_table() {
        let names: Vec<&str> = ANALYSIS_STAGES.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "goal",
                "implementation",
                "execution-flow",
                "api-compat",
                "reconciliation",
                "concurrency",
                "resources",
                "security",
                "tests",
            ]
        );

        for def in ANALYSIS_STAGES {
            assert!(
                def.guides.is_empty(),
                "{}: maintainer notes are selected from the diff, not per stage",
                def.name
            );
        }

        for core in ["goal", "implementation", "execution-flow"] {
            assert!(!analysis_stage_by_name(core).unwrap().optional, "{core}");
        }
        for optional in [
            "api-compat",
            "reconciliation",
            "concurrency",
            "resources",
            "security",
            "tests",
        ] {
            assert!(
                analysis_stage_by_name(optional).unwrap().optional,
                "{optional}"
            );
        }
    }

    #[test]
    fn test_kubernetes_stage_lookup_and_labels() {
        assert_eq!(stage_short_label("Execution_Flow"), Some("Execution Flow"));
        assert_eq!(stage_short_label("api-compat"), Some("API & Compat"));
        assert_eq!(stage_short_label("API_Compat"), Some("API & Compat"));
        assert_eq!(stage_short_label("reconciliation"), Some("Reconciliation"));
        assert_eq!(
            stage_short_label("post-verification-3"),
            Some("Post-Verification")
        );
        assert_eq!(stage_short_label("report"), Some("Report Generation"));
        // Stages of sibling workflows are not stages here.
        assert_eq!(stage_short_label("configuration"), None);
        assert_eq!(stage_short_label("summary"), None);
    }

    #[test]
    fn test_build_kubernetes_patch_review_workflow() {
        let wf = build_kubernetes_patch_review_workflow();
        assert_eq!(wf.name, "kubernetes_patch_review");
        // analysis fan-out, no-findings guard, verification fan-out,
        // no-findings guard, report
        assert_eq!(wf.steps.len(), 5);
    }

    #[test]
    fn test_kubernetes_notes_for_an_api_type_patch() {
        let selected = selected_notes(&["staging/src/k8s.io/api/core/v1/types.go"], "");
        assert_notes(
            &selected,
            &["general", "api-changes"],
            &["api-validation", "controllers", "kubelet", "tests"],
        );

        let selected = selected_notes(&["pkg/apis/core/validation/validation.go"], "");
        assert_notes(&selected, &["api-changes", "api-validation"], &[]);

        for file in [
            "pkg/registry/core/pod/strategy.go",
            "pkg/api/pod/util.go",
            "pkg/apis/batch/v1/defaults.go",
            "staging/src/k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/validation/validation.go",
        ] {
            let selected = selected_notes(&[file], "");
            assert_notes(&selected, &["api-validation"], &[]);
        }
    }

    #[test]
    fn test_kubernetes_notes_for_a_controller_patch() {
        let selected = selected_notes(
            &["pkg/controller/job/job_controller.go"],
            "@@ -10,2 +10,3 @@\n+\tlogger := klog.FromContext(ctx)\n",
        );
        assert_notes(
            &selected,
            &["general", "controllers", "compatibility", "logging"],
            &["kubelet", "security", "api-changes", "metrics"],
        );
    }

    #[test]
    fn test_kubernetes_notes_load_by_symbol_outside_their_directories() {
        // A workqueue or a feature gate is used all over the tree, so the
        // identifier must load the note where no path does.
        let selected = selected_notes(
            &["pkg/volume/foo/foo.go"],
            "@@ -1,2 +1,3 @@\n+\t\tc.queue.AddRateLimited(key)\n",
        );
        assert_notes(&selected, &["controllers"], &["feature-gates"]);

        let selected = selected_notes(
            &["pkg/volume/foo/foo.go"],
            "@@ -1,2 +1,3 @@\n+\tif utilfeature.DefaultFeatureGate.Enabled(features.Foo) {\n",
        );
        assert_notes(&selected, &["feature-gates"], &["controllers"]);

        let selected = selected_notes(
            &["pkg/volume/foo/foo.go"],
            "@@ -1,2 +1,3 @@\n+\tlegacyregistry.MustRegister(fooTotal)\n",
        );
        assert_notes(&selected, &["metrics"], &[]);
    }

    #[test]
    fn test_kubernetes_notes_for_component_and_support_paths() {
        let selected = selected_notes(&["pkg/kubelet/kubelet_test.go"], "");
        assert_notes(&selected, &["kubelet", "tests", "compatibility"], &[]);

        let selected = selected_notes(&["test/e2e/network/service.go"], "");
        assert_notes(&selected, &["tests"], &["kubelet"]);

        let selected = selected_notes(&["pkg/kubelet/metrics/metrics.go"], "");
        assert_notes(&selected, &["metrics", "kubelet"], &[]);

        let selected = selected_notes(&["plugin/pkg/auth/authorizer/node/node_authorizer.go"], "");
        assert_notes(&selected, &["security"], &["kubelet"]);

        for file in [
            "go.mod",
            "staging/src/k8s.io/client-go/go.mod",
            "vendor/golang.org/x/net/http2/server.go",
        ] {
            let selected = selected_notes(&[file], "");
            assert_notes(&selected, &["dependencies"], &[]);
        }
    }

    #[test]
    fn test_kubernetes_notes_do_not_load_for_ordinary_go_code() {
        // Every import of client-go contains the token "go", and most Go
        // code names ctx and err. None of them may select a note.
        let selected = selected_notes(
            &["pkg/util/foo/foo.go"],
            "@@ -1,3 +1,6 @@\n+import \"k8s.io/client-go/kubernetes\"\n+\tgo func() {\n+\t\tif err != nil {\n+\t\t\treturn ctx.Err()\n+\t\t}\n+\t}()\n",
        );
        assert_eq!(selected, vec!["general".to_string()]);
    }

    #[test]
    fn test_kubernetes_severity_guide_is_bundled_for_the_project() {
        let guide = crate::prompt_bundle::severity_guide(crate::project::ProjectId::Kubernetes);
        assert!(guide.contains("# Severity Levels"));
        assert!(guide.contains("alpha gate"));
    }

    #[test]
    fn test_kubernetes_maintainer_notes_render_before_code_notes() {
        let rendered = kubernetes_system_prompt(true).render_for_log(&KubernetesPatchReviewState {
            maintainer_notes: "\n\n<maintainer_notes>M</maintainer_notes>".to_string(),
            code_notes: "\n\n<code_notes>C</code_notes>".to_string(),
            ..Default::default()
        });
        let m = rendered.find("<maintainer_notes>M").unwrap();
        let c = rendered.find("<code_notes>C").unwrap();
        let guidelines = rendered.find("</global_review_guidelines>").unwrap();
        assert!(guidelines < m && m < c);
    }

    #[test]
    fn test_kubernetes_stages_carry_the_review_directives() {
        for required in [
            "the candidate concern that was investigated and disproved",
            "NO DISMISSAL WITHOUT VERIFIED PROOF",
            "MUST cite the concrete disproving code",
            "GENERATED AND VENDORED FILES",
        ] {
            assert!(
                CONCERN_JSON_SCHEMA_EXAMPLE.contains(required),
                "Kubernetes analysis stage guidance lost: {required}"
            );
        }
        for def in ANALYSIS_STAGES.iter().filter(|def| def.name != "goal") {
            assert!(
                def.instruction
                    .contains("Do not stop after finding several"),
                "Kubernetes {} stage must include full-hunk sweep directive",
                def.name
            );
        }
        assert!(
            STAGE_VERIFICATION_INSTRUCTION.contains("speculative_dismissal")
                && STAGE_VERIFICATION_INSTRUCTION
                    .contains("Single-caller or happy-path-only proofs")
                && STAGE_VERIFICATION_INSTRUCTION
                    .contains("Validation or feature gate cited without proof"),
            "Kubernetes verification stage must classify speculative dismissals"
        );
        assert!(
            STAGE_POST_VERIFICATION_INSTRUCTION.contains("SYMMETRICAL PROOF BAR")
                && STAGE_POST_VERIFICATION_INSTRUCTION.contains("ALL-CALLERS VERIFICATION")
                && STAGE_POST_VERIFICATION_INSTRUCTION
                    .contains("FEATURE GATE AND VERSION SKEW RULE"),
            "Kubernetes post-verification stage must enforce the proof bar"
        );
    }

    #[test]
    fn test_kubernetes_does_not_audit_signed_off_by() {
        // Kubernetes requires a CLA, not DCO. Sibling workflows report a
        // missing trailer at High severity; here every mention must forbid
        // the report instead.
        assert!(STAGE_GOAL_INSTRUCTION.contains(
            "NEVER raise a concern about a missing or a present `Signed-off-by` trailer"
        ));
        assert!(STAGE_POST_VERIFICATION_INSTRUCTION.contains(
            "Dismiss any candidate about a missing or a present `Signed-off-by` trailer"
        ));
        for instruction in ANALYSIS_STAGES.iter().map(|def| def.instruction).chain([
            STAGE_VERIFICATION_INSTRUCTION,
            STAGE_POST_VERIFICATION_INSTRUCTION,
            STAGE_REPORT_INSTRUCTION,
            CONCERN_JSON_SCHEMA_EXAMPLE,
        ]) {
            assert!(!instruction.contains("Real Name"));
            assert!(!instruction.contains("DCO"));
        }
        let severity = crate::prompt_bundle::severity_guide(crate::project::ProjectId::Kubernetes);
        assert!(!severity.contains("Signed-off-by"));
    }

    #[test]
    fn test_ci_noise_is_discarded_and_logic_errors_are_not() {
        assert!(STAGE_VERIFICATION_INSTRUCTION.contains("generated-code noise"));
        assert!(STAGE_POST_VERIFICATION_INSTRUCTION.contains("`make verify`"));
        assert!(
            STAGE_POST_VERIFICATION_INSTRUCTION
                .contains("A logic error that passes those checks is not such an error")
        );
    }

    #[test]
    fn test_series_context_enabled_for_kubernetes_wiring_and_verification_stages() {
        // Generated code, gate wiring, and tests often arrive in a later
        // commit of the same pull request.
        for stage_name in ["goal", "implementation", "api-compat", "tests"] {
            let def = analysis_stage_by_name(stage_name).expect(stage_name);
            assert!(
                def.wants_series_context,
                "{stage_name} should have wants_series_context enabled"
            );
        }
        assert!(VERIFICATION.wants_series_context);
        assert!(POST_VERIFICATION.wants_series_context);
    }

    #[test]
    fn test_kubernetes_verification_rejects_malformed_output() {
        let state = KubernetesPatchReviewState::default();
        let stage = verification_stage(20, 0.0);
        assert!(stage.output_format.validate("{}", &state).is_err());
        assert!(
            stage
                .output_format
                .validate(
                    r#"{"findings": [{"problem": "kubelet: missing nil check", "severity": "High", "severity_explanation": "Optional field", "preexisting": false, "locations": []}], "inconclusive_concerns": [], "dismissed_concerns": []}"#,
                    &state
                )
                .is_ok()
        );
    }

    fn response(body: String) -> crate::ai::AiResponse {
        crate::ai::AiResponse {
            content: Some(body),
            thought: None,
            thought_signature: None,
            tool_calls: None,
            usage: None,
            truncated: false,
        }
    }

    fn capabilities() -> crate::ai::ProviderCapabilities {
        crate::ai::ProviderCapabilities {
            model_name: "mock".to_string(),
            context_window_size: 1000,
        }
    }

    fn prompt_text(request: &crate::ai::AiRequest) -> String {
        request
            .messages
            .iter()
            .filter_map(|m| m.content.as_deref())
            .collect()
    }

    fn workflow_env(
        provider: std::sync::Arc<dyn crate::ai::AiProvider>,
        dir: &tempfile::TempDir,
    ) -> crate::workflow::stage::WorkflowEnv<'static> {
        crate::workflow::stage::WorkflowEnv {
            provider,
            tools: std::sync::Arc::new(crate::toolbox::ToolBox::new(dir.path().to_path_buf())),
            base_dir: std::path::Path::new(PROMPTS_DIR),
            context_tag: None,
        }
    }

    #[tokio::test]
    async fn test_workflow_runs_planning_and_core_stages_on_a_clean_patch() {
        struct CleanProvider {
            prompts: std::sync::Mutex<Vec<String>>,
        }

        #[async_trait::async_trait]
        impl crate::ai::AiProvider for CleanProvider {
            async fn generate_content(
                &self,
                request: crate::ai::AiRequest,
            ) -> anyhow::Result<crate::ai::AiResponse> {
                self.prompts.lock().unwrap().push(prompt_text(&request));
                let body = if request.response_format.is_some() {
                    r#"{"relevant_stages": []}"#
                } else {
                    r#"{"concerns": [], "dismissed_concerns": []}"#
                };
                Ok(response(body.to_string()))
            }

            fn get_capabilities(&self) -> crate::ai::ProviderCapabilities {
                capabilities()
            }
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let provider = std::sync::Arc::new(CleanProvider {
            prompts: std::sync::Mutex::new(Vec::new()),
        });
        let env = workflow_env(provider.clone(), &temp_dir);

        let wf = build_kubernetes_patch_review_workflow_with_options(1, 0.0);
        let mut state = KubernetesPatchReviewState::default();
        crate::workflow::WorkflowEngine::execute(&wf, &env, &mut state, None)
            .await
            .unwrap();

        // planning + goal, implementation, execution-flow; the empty analysis
        // results exit before verification and report.
        assert_eq!(provider.prompts.lock().unwrap().len(), 4);

        for manual in [
            None,
            Some(vec!["api-compat".to_string(), "reconciliation".to_string()]),
        ] {
            provider.prompts.lock().unwrap().clear();
            let mut state = KubernetesPatchReviewState {
                cheap_mode: true,
                manual_stages: manual.clone(),
                ..Default::default()
            };
            crate::workflow::WorkflowEngine::execute(&wf, &env, &mut state, None)
                .await
                .unwrap();
            let prompts = provider.prompts.lock().unwrap();
            assert_eq!(prompts.len(), 1, "cheap mode must skip planning");
            for def in ANALYSIS_STAGES {
                let selected = manual
                    .as_ref()
                    .is_none_or(|stages| stages.iter().any(|s| s == def.name));
                assert_eq!(
                    prompts[0].contains(&format!("<analysis_domain name=\"{}\"", def.name)),
                    selected,
                    "{}",
                    def.name
                );
            }
        }
        assert_eq!(
            stage_short_label("analysis"),
            Some(COMBINED_ANALYSIS_SHORT_LABEL)
        );
        assert_eq!(
            crate::workflows::planned_stages_from(
                crate::project::ProjectId::Kubernetes,
                &["analysis"]
            ),
            ["analysis", "verification", "post-verification", "report"]
        );
    }

    #[tokio::test]
    async fn partial_analysis_and_post_verification_failures_abort_review() {
        struct FaultProvider {
            fail_post: bool,
            successful_analysis: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl crate::ai::AiProvider for FaultProvider {
            async fn generate_content(
                &self,
                request: crate::ai::AiRequest,
            ) -> anyhow::Result<crate::ai::AiResponse> {
                let prompt = prompt_text(&request);
                let body = if request.response_format.is_some() {
                    json!({"relevant_stages": []})
                } else if prompt.contains("# Per-finding post-verification") {
                    if prompt.contains("FAIL_POST_CANDIDATE") {
                        anyhow::bail!("injected post-verification failure");
                    }
                    json!({"findings": [], "dismissed_concerns": [{
                        "source_ids": ["I2"], "type": "Validation",
                        "description": "PASS_POST_CANDIDATE",
                        "reasoning": "The caller rejects the nil object.",
                        "locations": [{"file": "pkg/foo/foo.go", "function_or_symbol": "validate",
                            "code_snippet": "if obj == nil { return err }", "side": "REVISION",
                            "why_this_location_matters": "The guard rejects the nil object."}]
                    }]})
                } else if prompt.contains("# Verification and severity estimation") {
                    let candidates: Vec<Value> = ["FAIL_POST_CANDIDATE", "PASS_POST_CANDIDATE"]
                        .iter().enumerate().map(|(index, description)| json!({
                            "source_ids": [format!("C{}", index + 1)],
                            "type": "Validation", "description": description,
                            "estimated_severity": "Medium", "signal_reason": "other",
                            "concern_arguments": "The object may be nil.",
                            "dismissal_arguments": "", "verification_question": "Is it checked?",
                            "preexisting": false, "locations": []
                        })).collect();
                    json!({"findings": [], "inconclusive_concerns": candidates, "dismissed_concerns": []})
                } else if prompt.contains("# Analyze commit main goal") {
                    if !self.fail_post {
                        anyhow::bail!("injected analysis failure");
                    }
                    let concerns: Vec<Value> = ["FAIL_POST_CANDIDATE", "PASS_POST_CANDIDATE"]
                        .iter()
                        .map(|description| {
                            json!({
                                "type": "Validation", "description": description,
                                "reasoning": "The object may be nil.",
                                "preexisting": false, "locations": []
                            })
                        })
                        .collect();
                    json!({"concerns": concerns, "dismissed_concerns": []})
                } else {
                    self.successful_analysis
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    json!({"concerns": [], "dismissed_concerns": []})
                };
                Ok(response(body.to_string()))
            }

            fn get_capabilities(&self) -> crate::ai::ProviderCapabilities {
                capabilities()
            }
        }

        for (fail_post, cheap_mode) in [(false, false), (true, false), (false, true)] {
            let dir = tempfile::tempdir().unwrap();
            let provider = std::sync::Arc::new(FaultProvider {
                fail_post,
                successful_analysis: std::sync::atomic::AtomicUsize::new(0),
            });
            let env = workflow_env(provider.clone(), &dir);
            let workflow = build_kubernetes_patch_review_workflow_with_options(1, 0.0);
            let mut state = KubernetesPatchReviewState {
                cheap_mode,
                ..Default::default()
            };
            let error = crate::workflow::WorkflowEngine::execute(&workflow, &env, &mut state, None)
                .await
                .expect_err("a partial stage failure must not produce a clean review");
            let failed_stage = if fail_post {
                "post-verification"
            } else if cheap_mode {
                "analysis"
            } else {
                "goal"
            };
            assert!(format!("{error:#}").contains(failed_stage), "{error:#}");
            if !cheap_mode {
                assert!(
                    provider
                        .successful_analysis
                        .load(std::sync::atomic::Ordering::SeqCst)
                        > 0
                );
            }
            if fail_post {
                assert_eq!(
                    state.post_verification_dismissed.len(),
                    1,
                    "the other post-verification batch must have succeeded"
                );
            }
        }
    }

    /// Plays a review with two concerns: verification confirms one and leaves
    /// the other inconclusive, and post-verification confirms that one too.
    struct TwoConcernProvider {
        stages: std::sync::Mutex<Vec<&'static str>>,
    }

    #[async_trait::async_trait]
    impl crate::ai::AiProvider for TwoConcernProvider {
        async fn generate_content(
            &self,
            request: crate::ai::AiRequest,
        ) -> anyhow::Result<crate::ai::AiResponse> {
            let prompt = prompt_text(&request);
            let (stage, body) = if request.response_format.is_some() {
                ("planning", r#"{"relevant_stages": []}"#)
            } else if prompt.contains("# Generate plain-text inline review report") {
                (
                    "report",
                    "- [HIGH] In pkg/controller/foo/foo.go, the optional field is not checked.",
                )
            } else if prompt.contains("# Per-finding post-verification") {
                (
                    "post-verification",
                    r#"{"findings": [{"source_ids": ["I1"], "problem": "controller: foo: lock held across API call", "severity": "Medium", "severity_explanation": "Stalls every worker.", "preexisting": false, "locations": []}], "dismissed_concerns": []}"#,
                )
            } else if prompt.contains("# Verification and severity estimation") {
                (
                    "verification",
                    r#"{"findings": [{"source_ids": ["C1"], "problem": "controller: foo: missing nil check", "severity": "High", "severity_explanation": "The field is optional.", "preexisting": false, "locations": []}], "inconclusive_concerns": [{"source_ids": ["C2"], "type": "Locking / Race", "description": "lock held across API call", "estimated_severity": "Medium", "signal_reason": "The callee may block.", "concern_arguments": "A callee may block.", "dismissal_arguments": null, "verification_question": "Does the callee block?", "preexisting": false, "locations": []}], "dismissed_concerns": []}"#,
                )
            } else if prompt.contains("# Trace execution flow") {
                (
                    "execution-flow",
                    r#"{"concerns": [{"type": "Nil Dereference", "description": "missing nil check", "reasoning": "The field is optional.", "preexisting": false, "locations": []}, {"type": "Locking / Race", "description": "lock held across API call", "reasoning": "A callee may block.", "preexisting": false, "locations": []}], "dismissed_concerns": []}"#,
                )
            } else {
                ("analysis", r#"{"concerns": [], "dismissed_concerns": []}"#)
            };
            self.stages.lock().unwrap().push(stage);
            Ok(response(body.to_string()))
        }

        fn get_capabilities(&self) -> crate::ai::ProviderCapabilities {
            capabilities()
        }
    }

    async fn run_two_concern_review(
        skip_report: bool,
        cheap_mode: bool,
    ) -> (KubernetesPatchReviewState, Vec<&'static str>) {
        let temp_dir = tempfile::tempdir().unwrap();
        let provider = std::sync::Arc::new(TwoConcernProvider {
            stages: std::sync::Mutex::new(Vec::new()),
        });
        let env = workflow_env(provider.clone(), &temp_dir);
        let wf = build_kubernetes_patch_review_workflow_with_options(1, 0.0);
        let mut state = KubernetesPatchReviewState {
            skip_report,
            cheap_mode,
            ..Default::default()
        };
        crate::workflow::WorkflowEngine::execute(&wf, &env, &mut state, None)
            .await
            .unwrap();
        let stages = provider.stages.lock().unwrap().clone();
        (state, stages)
    }

    #[tokio::test]
    async fn test_workflow_reports_concerns_confirmed_by_either_verification_stage() {
        let (state, stages) = run_two_concern_review(false, false).await;

        assert_eq!(
            stages
                .iter()
                .filter(|s| ["verification", "post-verification", "report"].contains(s))
                .copied()
                .collect::<Vec<_>>(),
            vec!["verification", "post-verification", "report"]
        );
        let problems: Vec<&str> = state
            .findings
            .iter()
            .filter_map(|f| f["problem"].as_str())
            .collect();
        assert_eq!(
            problems,
            vec![
                "controller: foo: missing nil check",
                "controller: foo: lock held across API call",
            ]
        );
        assert!(state.review_inline.contains("[HIGH]"));
    }

    #[tokio::test]
    async fn test_skip_report_keeps_findings_and_omits_the_report_stage() {
        let (state, stages) = run_two_concern_review(true, true).await;

        assert_eq!(
            stages,
            ["execution-flow", "verification", "post-verification"]
        );
        assert_eq!(state.findings.len(), 2);
        assert!(state.review_inline.is_empty());
    }
}
