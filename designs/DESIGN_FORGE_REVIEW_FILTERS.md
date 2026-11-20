# Design: Forge Review Filters

## 1. Problem

A Sashiko daemon with `forge.enabled = true` reviews every pull request that a
webhook reports or that the Pulls API poller finds. Only Dependabot is skipped.

That is right for an instance that reviews its own repository. It does not
work for an operator who wants reviews for part of a busy repository. An
example is a maintainer who runs a private instance with `post_mode =
"dry_run"` against `kubernetes/kubernetes`: that repository has more than a
thousand open pull requests, and more than a hundred of them change each day.
Each change costs a full review per commit.

## 2. Goals and non-goals

Goals:

1. Let the operator limit automatic review to pull requests of listed authors.
2. Let the operator limit automatic review to pull requests that carry a listed
   label.
3. Apply the same rule to webhooks and to the poller.
4. Keep today's behavior when nothing is configured.

Non-goals:

- A filter on changed paths. The file list is known only after the commits are
  fetched, which is later than the point where a pull request is accepted.
- Filters on assignees, requested reviewers, or draft state.
- A separate filter for each repository in `forge.repos`.
- A filter on manual submission. `POST /api/submit`, `sashiko-cli submit`, and
  local review are actions of the operator and stay unfiltered.

## 3. Configuration

```toml
[forge]
# Review only pull requests opened by these logins.
authors = ["octocat"]
# Review only pull requests that carry at least one of these labels.
labels = ["sig/network", "needs-sashiko"]
```

Both keys accept a TOML array or a comma-separated string, as `repos` does, so
`SASHIKO__FORGE__AUTHORS` and `SASHIKO__FORGE__LABELS` work.

Rules:

- An empty list does not restrict. With both lists empty, every pull request is
  reviewed, as before.
- Inside one list, one match is enough. When both lists are set, the pull
  request must match both.
- Logins and labels are compared without regard to case, after trimming.
- With `authors` set, a pull request whose author is not known is not
  reviewed. The filter exists to bound cost, so it fails closed.
- The Dependabot rule is independent and stays.

## 4. Design

### 4.1 Metadata

`ForgeMetadata` gains `labels: Vec<String>`:

| Source | Field |
|---|---|
| GitHub `pull_request` webhook | `pull_request.labels[].name` |
| GitHub Pulls API | `labels[].name` of each item |
| GitLab merge request webhook | `labels[].title`, else `object_attributes.labels[].title` |

A missing or malformed label list is an empty list. It never rejects a payload:
labels are optional data, and an instance without a label filter must not start
to refuse deliveries because of them.

On GitLab, `author` is the user who caused the event. That is the author when
a merge request is opened and when the author pushes. A push by another user to
the same merge request is judged by that user. GitLab parsing has this limit
today; this design does not change it.

### 4.2 The filter

`forge.rs` gets a small type so that the rule exists once:

```rust
pub enum ReviewFilterDecision {
    Review,
    AuthorNotListed,
    LabelNotListed,
}

pub struct ForgeReviewFilter { /* normalized authors and labels */ }

impl ForgeReviewFilter {
    pub fn from_settings(settings: &ForgeSettings) -> Self;
    pub fn decide(&self, metadata: &ForgeMetadata) -> ReviewFilterDecision;
}
```

The decision is an enum, not a bool, so that the log line and the webhook
answer can say which list excluded the pull request.

### 4.3 Where it applies

- **Webhook** (`server.rs`): after the review-intent check and the Dependabot
  check. A filtered pull request gets `200` with `"status": "ignored"` and the
  reason, like the other ignored cases. A forge shows non-2xx answers as failed
  deliveries, and a filtered pull request is not a failure.
- **Poller** (`worker/forge.rs`): next to the Dependabot check. A filtered item
  is skipped and does not count as an error, so the watermark still advances.

### 4.4 Labels that arrive later

A label is often added after a pull request is opened.

- With polling on (the default), this needs nothing new. A label change
  updates `updated_at`, the poller sees the pull request again above its
  watermark, the head was never ingested, and the pull request now passes.
- With `poll_interval_secs = 0`, a pull request that gets its label after its
  last push is reviewed at its next push. The `labeled` action stays a skipped
  action: its payload carries a base commit that moves with the target branch,
  and treating it as new work would review unchanged code again each time any
  label changes. The documentation tells operators to keep polling on when
  they filter by label.

### 4.5 Changing the filter

The poller looks only at pull requests updated after its watermark. A pull
request that an old filter excluded is therefore reviewed after a filter change
only when it is updated again. This is the cheap and predictable choice: a
wider filter never starts a review of the whole backlog.

## 5. Tests

- Unit tests for the decision: each list alone, both lists, case and
  whitespace, unknown author, empty lists.
- Label extraction from a GitHub webhook payload, a Pulls API item, and a
  GitLab payload, including a payload with no labels.
- Poller test with the mock Pulls API: only matching pull requests are queued,
  and a pull request that gets its label later is queued on the next pass.
- Webhook test: a filtered pull request is answered with `ignored` and no
  patchset is created.
- Settings test: array and comma-separated forms.
