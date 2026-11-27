# RelAI audit — 370ef70

## Project graph

5. [ ] **Older snapshots become unreachable from the selectors.** Capture 65 snapshots and select the latest. Snapshot 1 is absent from the browse and comparison choices; expect a way to reach it without already knowing its ID. Source: `app/graph_snapshots.sql` (latest 64 plus the current selection), `app/graph.fluxr` (shared choices).
8. [ ] **Rust path overrides are missing from the relationship graph.** Put `#[path = "actual.rs"] mod child;` in `src/lib.rs` and capture both `src/actual.rs` and an unused `src/child.rs`. The analyzer classifies attributed external declarations as `module`, not `external_module`, so the graph skips this declaration. Expect a link to the retained `src/actual.rs` and no link to the unused `src/child.rs`. Source: `runtime/src/syntax.rs` (`visit_item`), `app/graph_module_relationships.sql` (selects only `external_module`).
9. [ ] **Custom Rust crate roots are treated as nested modules.** Set `[lib] path = "src/entry.rs"` in Cargo.toml, declare `mod child;` in `src/entry.rs`, and put the child in `src/child.rs`. Inspect the snapshot. The relationship lookup searches under `src/entry/` and misses the child; expect a link to `src/child.rs`. Source: `app/graph_module_relationships.sql` (`module_directories` recognizes only `main.rs`, `lib.rs` and `mod.rs`).

## Dictionary, Skills, Models, and Tools


## Providers, delivery, and launcher

22. [ ] **Retry delivery stays available after the attempt limit.** Once a job has three attempts, the retry control can requeue the same job, but the scheduler holds it again without sending. Expect an exhausted state with no retry button for that job. Source: `app/chat.fluxr` (`chat_delivery_options`), `app/receive.fluxr` (`control`), `app/queue.fluxr` (`poll`).
23. [ ] **A fresh attempt does not recover an exhausted request.** At the 64-call request limit, Start a fresh attempt creates a replacement job on the same request, so the request-wide count still blocks it. The count also spans request revisions. Expect the UI to explain the limit and offer a path that can actually run. Source: `app/chat.fluxr` (`chat_delivery_options`), `app/receive.fluxr` (`control`), `app/queue.fluxr` (`poll`).
24. [ ] **Ambiguous delivery warnings are inconsistent before and after restart.** A transport error can leave a held job with an uncertain attempt. `hold` uses a missing `body_received` value in its SQL condition, and the chat warns of possible duplicate delivery for uncertain attempts. However, a seal-time failure has no attempt at all and is shown as a model response failure. Expect the status to distinguish preparation failure from an uncertain send. Source: `runtime/src/main.rs` (`work`), `app/queue.fluxr` (`hold`), `app/chat_progress.sql`.

## Attachments, file ranges, and Tools


## Format edge cases


## Git status, changes, and publication

43. [ ] **Ignored build outputs can make Git status fail.** In an otherwise small checkout, add an ignored build output over 8 MiB and request status. The raw worktree scan counts it and rejects the scan; expect ignored outputs to be excluded or status to report explicitly that it is incomplete. Source: `runtime/src/git_operations.rs` (`inspect`).
44. [ ] **Git status misses executable-bit-only edits.** On a filesystem that supports executable bits, change only the executable bit of a tracked file and compare RelAI status with Git status with `core.fileMode=true`. RelAI compares only the blob ID for worktree changes. Source: `runtime/src/git_operations.rs` (`inspect`).
45. [ ] **Commit apply does not recheck unfinished-operation markers.** Prepare a selected-path commit, then start a no-commit merge that leaves HEAD and the index unchanged, or use an isolated fixture that adds only `MERGE_HEAD`. The marker check is in preparation, not apply; expect the approved commit to stop rather than move the branch while the merge is pending. Source: `runtime/src/git_operations.rs` (`prepare`, `apply`).
46. [ ] **A selected-path commit can be empty.** Propose a commit selecting only a tracked path whose staged content and mode already match HEAD. The code constructs a commit with the unchanged tree; expect a no-change result unless an empty commit was requested explicitly. Source: `runtime/src/git_operations.rs` (`prepare`, commit branch).
47. [ ] **Remote registration accepts a non-HTTPS URL.** Register a remote using a nonempty HTTP URL and a full branch ref. Registration returns an ID and lists the remote as active despite the HTTPS contract; expect rejection before the record is saved. Source: `app/remotes.fluxr` (`remote`).
48. [ ] **Remote authorization accepts a malformed commit ID of the right length.** With a valid registered remote, enter 40 non-hex characters as the target. Authorization checks length only and returns an operation ID; expect rejection before recording approval. Source: `app/remotes.fluxr` (`remote`, authorize branch).
50. [ ] **An interrupted fetch can remain uncertain indefinitely.** When a dispatched fetch ends with a transport error, the completion handler records `uncertain`. The reconciliation operation accepts only uncertain pushes, leaving no reconciliation path for that fetch. Expect a way to resolve the fetch or retire its uncertain outcome explicitly. Source: `app/remotes.fluxr` (`remote`, `guest_transfer_done`).
51. [ ] **Publication cannot use a newly discovered defect in an already active baseline.** Qualify a baseline, then demonstrate a defect outside the built-in fixtures and supply a corrected version. Author tests do not create verified checks, and qualification only tests drafts against the built-in fixtures. Publication still requires a verified failing baseline case; expect a reviewed way to add that case without treating author evidence as independent. Source: `app/publication.fluxr` (`publish_candidate`), `app/tools.fluxr` (`tool_test_start`, `guest_tool_tested`).

## Schedules and monitors


## Connectors

57. [ ] **Connector registration accepts invalid destinations.** Register a GET connector with a nonempty malformed URL and a valid argument schema. It is listed as active; the URL is parsed only at execution. Expect registration to validate the destination. Source: `app/connectors.fluxr` (`connector`), `runtime/src/network.rs` (`exchange`).
58. [ ] **Unsupported idempotency header names are accepted at registration.** Register a connector with an unsupported header name. Registration succeeds, but execution rejects it before sending; expect the two supported header names to be enforced at registration. Source: `app/connectors.fluxr` (`connector`), `runtime/src/network.rs` (`exchange`).
59. [ ] **A missing connector credential is reported as an uncertain send.** Use a connector with a valid credential reference whose environment variable is unset. The host fails before calling `send`, but the completion handler records `uncertain`; expect a configuration failure that clearly says no request was sent. Source: `runtime/src/network.rs` (`exchange`), `runtime/src/main.rs` (`invoke`), `app/connectors.fluxr` (`guest_net_done`).
60. [ ] **A definite GET error loses its HTTP outcome classification.** Have an approved test connector return HTTP 404. The status survives in error text, but the operation is marked `uncertain`; expect a received HTTP error to be distinct from a lost response. Source: `runtime/src/network.rs` (`exchange`), `app/connectors.fluxr` (`guest_net_done`).

## Semantic search

61. [ ] **The first search can silently miss files in a fresh snapshot.** Capture more than 128 eligible files with distinct digests and put the only match in the contents of a file outside the first digest batch. The first search can return no passages and a later identical search can find one. Expect a coverage indicator or a continuation. Source: `app/passages.fluxr` (`index_passages`, `ranked_passages`).
62. [ ] **The passage cap can permanently hide later short lines.** Use an eligible text file with over 2,048 short lines and place a unique term near the end. The chunk limit omits the tail, and the digest is still marked processed; repeat searches do not index the remainder. Source: `app/passages.fluxr` (`index_passages`).
63. [ ] **Semantic search misses non-ASCII case variants.** Place `Étage` in an eligible file and its pathname, then search for `étage` in the retained snapshot without a matching dictionary synonym. Path and passage ranking use SQLite `lower`; expect case handling consistent with the Unicode-aware dictionary suggestions. Source: `app/analysis.fluxr` (`semantic_find`), `app/passages.fluxr` (`ranked_passages`).
64. [ ] **A phrase spanning a passage boundary cannot match.** Put a unique phrase across the 512-character split of a long line, with no dictionary entry for its parts. Neither chunk contains the whole query, so the phrase is absent from results; expect a match across neighboring chunks or a documented limitation. Source: `app/passages.fluxr` (`index_passages`, `ranked_passages`).

## Skill package admission


## Desktop workspace and reassignment

71. [ ] **A permanently failed retained-file publication can block unrelated project work.** If publishing an unverified retained file repeatedly fails, `poll` encounters that file in its global publication loop before it can select any queued job or due occurrence. Expect the publication failure to be recorded and isolated so work in other projects can still run. Source: `app/queue.fluxr` (`poll`).

## Fresh baseline audit — additional findings

Static source review only; the triggers below were not executed. This pass records 29 additional findings, #72–#100.

73. [ ] **An unavailable saved project can still be selected as if it were available.** Register a project, close the app, move the folder, and relaunch with that project selected. Startup selection and project tabs use the stored ID and root without checking that the folder still exists. Expect a clear unavailable state and a way to locate or reopen the folder without losing its history. Source: `app/ui.fluxr` (`dashboard`, project selection and tab construction), `app/project_labels.sql` (saved project rows), `app/store.fluxr` (`project` versus `open_project`).
77. [ ] **A batched reply can be recorded as exact when it is only a substring.** For a question in a batch, compare a multiword user submission with an intake reply envelope that carries only one word from it. Admission tests containment, and sibling answer reconstruction repeats that containment check, so the batch can present the shortened payload as the answer. Expect the exact submitted text to be preserved and compared for equality. Source: `app/actions.fluxr` (`admit`, question-directed `MESSAGE`), `app/question_batch.sql` (answer lookup).
79. [ ] **An allowed attachment set can exhaust the seal arena after Send clears the draft.** Attach eight distinct 4 MiB plain ASCII text files and send a short message. Each file is below the native 8 MiB attachment ceiling, and submission checks count and prompt size but not total attachment size. During sealing, even just the first seven successively accumulated content arrays account for at least 112 MiB of serialized text across arena values (4 MiB × (1 + … + 7)), above the 96 MiB per-call limit; other intermediate values count too. The draft is cleared on successful submission, before sealing attempts to build the provider request. Expect an aggregate admission check before clearing the draft, or a bounded seal path that can handle every admitted attachment set. Source: `runtime/src/desktop.rs` (`capture`), `runtime/src/lib.rs` (`selected_file`, `Arena::add`), `app/store.fluxr` (`capture`, `submit`), `app/provider.fluxr` (`file_part`, `seal`), `app/desktop_events.fluxr` (`desktop_result`, draft-success branch).
