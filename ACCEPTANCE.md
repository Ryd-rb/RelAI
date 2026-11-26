# Acceptance status

The design is the product target. Each row gives the behavior covered by the current Windows integration suite and the boundaries of that evidence. The suite invokes checked FluxR exports, the loopback daemon, or the native primitive identified by the test.

| Design area | Implemented and tested | Remaining acceptance work |
|---|---|---|
| Relay, model turns and attachments | Permanent project identity, prompt and attachment capture, submission-key conflicts, immutable turn retry, user acceptance, final delivery, one improvement session | Full action/disposition coverage, cancellation during effects, verifier-backed criteria across every scope and fault-injection boundary |
| Routing and fan-outs | Separate human receipts for siblings, descendant review before an ancestor, restart while waiting on a named question, validation of review references | Mixed revision and replacement scenarios, all deadlock and wake cases, independent deterministic verification of every closure criterion |
| Local provider | One real ChatGPT-authenticated Codex App Server turn using a captured text attachment, exact model and effort, and an admitted result | Full live routing, review and improvement scenario |
| Other providers | Checked Anthropic Messages and Qwen Chat Completions request and response adapters | Endpoint behavior intentionally untested at the owner's request |
| Checked tools and improvement | Compiler admission, native/host rejection, traps and subsequent VM usability, immutable versions, independent counterexamples, qualification and retirement. A corrected Markdown link checker runs against retained snapshots without a model call | Broader contract families, automated dependent qualification and promotion, full improvement efficiency and regression gates |
| Files, caches and graph | SHA-256 artifacts, retained snapshot bytes, bounded line and byte reads, glob pagination, fresh host reads, project isolation, Windows watcher, in-memory LRU and snapshot reconciliation | Block-level I/O reuse, shared-job cancellation and complete single-flight accounting, architecture and activity layers, full graph interactions |
| Protected writes | Real NTFS TxF commits, stale expected-state rejection, copy/move/remove, atomic multi-file rollback, private paths and hard-link rejection, patch staging, exact grants and recovery | Packed refs and linked-worktree checkout guards, complete erasure and replay, additional crash boundaries |
| Git and source analysis | In-process object, tree and ref reads. Embedded Rust and FluxR syntax analysis with byte spans. Scoped query templates | Workspace-level symbol resolution, Smart HTTPS fetch and push, full index, ref, and worktree workflow |
| Connectors, schedules and monitors | Loopback fixture for GET and POST, immutable response artifacts, exact-argument authorization, duplicate and revoked operations. Reserved occurrences survive restart. Cancellation stops later dispatch | End-to-end remote failure and uncertain write reconciliation, complete authority/erasure matrix |
| Dictionary and skills | Linked lookups, exact revisions, UTF-8 span preservation, explicit ambiguity resolution, pinning of installed local package content and denial after removal | All definition navigation and historical-view cases, approved HTTPS Git package installation and update workflows |
| Document and structured data tools | DOCX, PDF text extraction, XLSX cells and formulas, notebooks, CSV and bounded ZIP processing. Archive expansion limits and malformed input are exercised | Full catalog input, output, resource-budget and access contracts |

## Reproduce

Run `pwsh -File .\Test-RelAI.ps1` from the checkout. The live subscription test is opt-in through `pwsh -File .\Start-RelAI.ps1 -CheckProvider`. It records the observed number of attempts and checks the word that came from the attachment. Offline tests use temporary data and make no inference calls.
