# RelAI implementation design

## Objective and scope

Build a local checked FluxR harness with one permanent relay per project. Implement RelAI behavior in FluxR. Extend the language and its reusable runtime services when the application needs a missing capability.

Open an existing project in its current local folder. Put its applicable project instruction files at the start of each model turn's input.

1. The relay writes one `PROMPT.txt` file for each submission.
2. The harness sends that file and its attachments to the project discriminator in the same model turn.
3. The discriminator supplies the contents of the continuation files. It specifies the actions and their destinations.
4. The harness validates the actions. It writes the continuation files before dispatch.
5. After each fan-out reaches a review point, the harness gives its outcomes to the discriminator.
6. The discriminator can send more `TASK` files or send one final `RESULT` file to close the request.
7. After the final result, the harness starts one improvement session for that request.

A worker can send a question to the relay. The discriminator prepares an addressed reply file. The harness gives the file to the worker.

Sessions can write and test FluxR tools. The daemon runs all tools in one embedded FluxR VM. A qualified tool can use new inputs without a model call.

The user can install skills and select them for an assignment. The project dictionary identifies the installed skill and its revision.

Use this document as the implementation prompt after the user gives permission to build RelAI. Update this document in place when the design changes.

RelAI is a local Windows application for one user. The native Windows relay UI runs in process. Checked FluxR defines its controls, state and interactions. The optional headless API uses a loopback connection. The daemon uses the full host access of the Windows account that starts it. Sessions use typed tool requests. The implementation must include every tool and acceptance check in this design.

The default provider uses the signed-in Codex App Server and ChatGPT subscription usage. Each sealed attempt opens an ephemeral thread and supplies its structured output schema. Anthropic Messages and self-hosted OpenAI-compatible Qwen Chat Completions are additional role-selectable providers. Use the structured output contract and attachment representations for the selected provider. The model reasons during each call. The harness controls all actions between calls. Each request uses only the sealed turn input. Start a fresh ephemeral thread for each Codex attempt. Send a fresh message request for each Anthropic or Qwen attempt. The application supplies only the sealed context and exposes no provider-hosted tools. [Codex App Server](https://developers.openai.com/codex/app-server) · [Anthropic Messages](https://platform.claude.com/docs/en/api/messages)

This document uses ASD-STE100 Issue 9 and the software terms in the next section. Identifiers, file names, quoted user input, and JSON examples keep their exact forms. [ASD-STE100 Issue 9](https://www.asd-ste100.org/STE_downloads.html)

## Dictionary of technical terms

Use one name for each item. The terms below also supply the first entries in the app's **RelAI** dictionary scope.

| Term | Definition |
|---|---|
| Relay | The permanent project chat. It shows submissions, questions, deliveries, status, and results. |
| Harness | The FluxR application that admits actions, dispatches jobs, assembles model input, and records results. |
| Daemon | The one application host process that owns the scheduler, the FluxR VM, and all tool services. |
| Submission | One user message with text, attachments, or both. Its `PROMPT.txt` contains the exact UTF-8 text. |
| Request | The work that the harness tracks together to give the user one final result. |
| Request revision | The version of the user's intent and acceptance criteria for a request. |
| Improvement session | The session that uses one completed request's evidence to improve future work in its project. |
| Improvement record | The project record of an observed problem, an exact candidate change, and the evidence for its outcome. |
| Attachment | An immutable copy of a file that the user adds to a submission. |
| Attachment manifest | The ordered record of the attachments and their exact artifact references. |
| Discriminator | The project session that routes submissions and reviews the outcomes of each fan-out. |
| Continuation file | A file that contains one typed action or one addressed delivery. |
| Action | A request in the RelAI protocol. Only the harness can admit an action. |
| Assignment | A unit of work with a deliverable, an acceptance criterion, and a scope revision. |
| Fan-out | One committed batch of new child assignments from a model turn for the same request and parent. |
| Fan-out branch | One child assignment in a fan-out and the work that descends from it. |
| Fan-out review | A discriminator turn that examines a fan-out's recorded outcomes and decides what to do next. |
| Scope | The project, component, or resources to which a record or permission applies. |
| Scope revision | The version of the instructions for an assignment. The protocol field `assignment_revision` holds this revision number. |
| Steering | A user instruction that changes work on an existing assignment. |
| Session | A durable address with an inbox. A session can wait between model turns. |
| Worker | A session with an assignment and a worker role. |
| Model role | The function that determines the instructions and action schema of a session. |
| Model preset | The model identifier and reasoning effort for one model role. |
| Project override | A model preset that replaces the default preset for one role in one project. |
| Model turn | A logical job with a fixed input and at most one accepted model outcome. |
| Turn seal | The commit that fixes the input and model preset for a model turn. |
| Inbox | The ordered list of deliveries for a session. |
| Question | An open request for input. It identifies its source and the work that needs the answer. |
| Delivery | An item for one session. It contains the exact payload, its source and destination, and its summary. |
| Scheduler | The part of the harness that starts eligible jobs and records their states. |
| Job | Work on a model turn, a tool test, or a tool invocation. |
| Attempt | One try to do a job. A retry uses a new attempt ID and the same job ID. |
| Prerequisite | A condition that must be true before an action can start. |
| Acceptance receipt | A record from a designated verifier that a specific acceptance criterion is satisfied. |
| Artifact | Immutable data or a file with a digest and an access scope. |
| Digest | A value calculated from bytes. The harness uses it to identify and compare exact content. |
| Snapshot | An immutable manifest of project paths and the digests of their retained bytes. |
| Project root | The local folder that the user opens as the project's working directory. |
| Project instruction file | An `AGENTS.md` or `CLAUDE.md` file from the project root or an applicable directory. |
| Host path | An explicit absolute Windows path to a live file or directory. |
| Watcher | The daemon service that receives filesystem change events for a registered directory. |
| Watched source | A registered live root or path with a watcher and a recorded generation. |
| Generation | An increasing number that marks the observed state of a watched source. |
| Single-flight | One queued or active job that supplies results to multiple equivalent tool requests. |
| Result cache | A daemon memory index of completed tool results and their source dependencies. |
| File cache | A daemon memory cache of file bytes and line indexes. It removes the least recently used entries first. |
| Project map | An index of project components, source references, verification commands, and decisions. |
| Project graph | The interactive display of project files, architecture evidence, and recorded activity. |
| Graph node | One identified project root, directory, file, or component in the project graph. |
| Graph edge | An identified connection between graph nodes. It records containment or a sourced relationship. |
| Dictionary | A named collection of terms that one project owns. Other projects can enable an explicit link. |
| Dictionary link | A project grant to read terms from a specified dictionary. |
| Dictionary entry | A named item of kind `knowledge`, `tool`, or `skill`. It has a stable ID and a revision. |
| Dictionary view | The enabled dictionaries and their recorded revision cursors for a lookup or model turn. |
| Term link | A reference from one definition to another term's stable ID. |
| Term marker | A structured link from an exact text span to a dictionary entry revision. |
| Skill | An installable package of instructions, references, and optional tool links or FluxR drafts. |
| Skill version | The exact package contents and source identity that the harness recorded at installation. |
| FluxR VM | The embedded Rust evaluator for checked FluxR tool code. The daemon gives it one tool job at a time. |
| Host service | A Rust function that the daemon makes available to FluxR through a typed interface. |
| Tool draft | Immutable FluxR source and its contract before qualification and activation. |
| Tool contract | The types of input and output, the supported inputs, and the capabilities necessary to run a tool. |
| Qualification | The checks that an exact tool version must pass to become eligible for activation. |
| Activation | The registry operation that makes a qualified tool version callable in its allowed scope. |
| Promotion | The registry operation that makes one active tool version the default. |
| Tool invocation | A typed request to run one exact active tool version with specified inputs. |
| Tool registry | The records of tool versions, qualification, activation, and default bindings. |
| Effect request | Typed data that identifies a requested change. The harness decides if it can make the change. |
| Effect receipt | A record of the expected state, actual result, and recovery state for a change. |
| Connector | A typed Rust interface to an authorized external data service. |
| Schedule | A persistent rule for when the daemon can start a specified action. |
| Tombstone | A record that identifies unavailable content without keeping its contents. |

In this document, **route** means select a destination. **Dispatch** means give admitted work to the destination. **Commit** means make a database transaction permanent. **Bind** means record an exact link between records. **Validate** means make sure that data obeys the specified rules. **Resolve** means identify the exact record, tool, or path for a reference. **Run** means start a tool in the FluxR VM. A function **returns** data when it sends its output to its caller.

## Records and files

### Project scope

Each project has one relay, one discriminator address, and one project map. It starts with an enabled dictionary that it owns. A project can own more dictionaries and enable links to other dictionaries. The project also has source snapshots, a tool registry, and a policy.

When the user opens an existing folder, use it as the project root. Resolve its canonical folder identity. Reopen the same project if the identity already exists. Use its live files and current checkout. Keep uncommitted changes. Opening a project does not copy its files, initialize Git, or create a worktree. Keep RelAI state in the daemon's data area. Use a separate worktree only when the harness dispatches work with that worktree explicitly selected.

Record the project scope for each operation. Use one writer for each project database. Serve private RelAI records only through their typed APIs. A project session can read shared terms under an enabled dictionary link.

An assignment can have a worker, a deterministic job, or both. Give each assignment its own acceptance criterion and scope revision. Independent assignments can continue after other work from the same submission finishes.

Create a request for new work that shares one completion goal. Give unrelated new work a separate request. Bind a reply or steering to its existing request. Record the source spans, acceptance criteria, and verifiers for each request. Keep request revisions and assignment scope revisions separate.

### Snapshot capture

Retain snapshot bytes in immutable storage before committing the manifest. Store the project-relative paths and verified digests in the manifest. Use the retained bytes for every snapshot read, including reads after the source disappears. An immutable Git source also records the commit and path. Keep absence and access errors distinct.

For a live source, capture bytes through stable file handles. Use a reconciled source generation and check identity and metadata before and after each read. Check for source changes during capture. If a change invalidates the capture, retry or report that the source changed. Record the capture interval and the observed generation. Use an admitted Git revision when the work needs a repository-wide version.

### File authority

Use SQLite as the source of scheduling state. Record the exact bytes of small prompts and continuation files with their action records. Write larger artifacts to immutable storage before the transaction records their references.

The relay writes the user's exact text to `PROMPT.txt`. Record its author, reply target, term selections, and idempotency key separately. Use an empty `PROMPT.txt` for a submission with files and no text. Repeated submission of the same key returns the stored submission ID. Reject reuse of the key with different content or metadata.

At the turn seal, record `SYSTEM.txt` and `input.json`. Record the effective model preset, its source revision, role, protocol version, scope revision, and exact inbox selection. Record the ordered instruction files, source excerpts, term references, and attachment representations in the input. Write both files before dispatch.

For a continuation, commit the action and any delivery record first. Record the expected path and digest for each file. Publish the bytes with a temporary file and an atomic rename. Verify the digest before dispatch. A committed delivery cannot start a turn until the harness verifies its required files. Use the same publication rule for turn and submission files.

If a file is missing or incorrect, recover it from its committed bytes under the same ID. First check that the record still permits publication. Never recover erased bytes. A file in an output directory cannot start work by itself. Accept submissions through the authenticated relay API.

### Attachments

Capture the file bytes when the user adds the file. Calculate a digest and store the bytes as an immutable artifact. Bind the artifact to the draft project. Commit `PROMPT.txt`, the attachment manifest, and submission metadata in one submission transaction. Publish the prompt and manifest, and verify the attachment bytes before the first discriminator request.

Always write `attachments.json`. Use an empty list when the submission has no attachments. For each file, record its ID, display name, media type, byte length, digest, and artifact ID. Preserve the order in the composer. Use the display name as data. Keep the original host path out of the model input.

Send all attachments in order with the prompt and manifest in the first discriminator request. Use inline `input_file` for supported documents and `input_image` for supported images. Use a text part for a supported UTF-8 file when it preserves the full content. Record the selected representation, serializer version, and artifact digest at the turn seal. A native file representation can give the model a provider extraction of the original bytes. Use typed artifact reads for requests that need exact bytes or data outside that extraction. [OpenAI file inputs](https://developers.openai.com/api/docs/guides/file-inputs) · [OpenAI image inputs](https://developers.openai.com/api/docs/guides/images-vision)

Check type, size, model support, and input limits before submission. Show the error for each affected file and keep the draft. Check the effective preset again at the turn seal. If the request cannot include a file, hold the turn and show the cause. Preserve a committed submission as sent. Let the user choose a compatible preset or make a new submission with different files.

Treat file contents as source data with no instruction authority. A routing action can name the specific attachment IDs that its destination needs. Check each ID against the submission and destination assignment. Give the worker those references and bounded content necessary for the task. Put exact bytes within reach of the worker through typed artifact reads. Other assignments receive no automatic copy.

### Identity and state

Give each admitted action, job, and delivery a stable ID. Bind references in a model response to harness IDs in the admission transaction. Record state changes and relay events in the same transaction.

Give each published project event an increasing cursor. After reconnection, send events after the UI's last cursor. If the cursor is unavailable, send a current projection and its cursor. Use indexed records for current state and pagination. Startup and routing do not need to read the full timeline.

Keep prompts, protocol files, dictionary revisions, test evidence, and tool receipts as product records. Archiving keeps the bytes and exact references. Exclusion from recall stops automatic selection. If the policy revokes access, return an unavailable outcome when a session requests the source.

Before file publication, a new provider attempt, or outcome admission, recheck the current authority for its exact input and destination. Check source access, the request revision, and the applicable assignment revision. Mark obsolete work as superseded. If access changed, stop the affected work and retire the sealed turn. A replacement turn must seal new inputs. Record an in-flight outcome that cannot be admitted without using its actions. Apply erasure to retained inputs and derived records under the erasure rules.

Erase content only when the user authorizes the exact scope. Make affected content unavailable before removing its bytes. Remove owned exports, indexes, caches, and derived records that contain the erased data. Keep a tombstone for references and delivery IDs. Resume an incomplete erasure after restart. Do not recreate erased files from previous events. Erasure applies to RelAI's local copies.

Reserve storage before admitting work. Keep space for state changes and the outcomes of admitted jobs. When capacity is insufficient, hold new work and show the cause. Account for drafts, staged bytes, and retained artifacts. Reclaim temporary bytes only when their owner is inactive and no committed record refers to them. Apply retention rules to committed bytes.

## Actions and model roles

### Action contract

Use one versioned JSON envelope for model actions. Each role receives the applicable action schema and instructions. For a submission, the discriminator supplies source references and a proposed disposition for each instruction. For a fan-out review, use the review contract.

The harness validates the complete envelope before it admits any action. The harness admits no actions from a malformed envelope, a refusal, or an incomplete response. The harness then checks each action against current state and its operation rules. For an invalid destination or a missing prerequisite, it holds only the affected action and its dependents.

| Action | Required data and effect |
|---|---|
| `TASK` | Include the worker role, assignment text, deliverable, acceptance criterion, constraints, source references, prerequisites, and summary. Include the skill versions that the user explicitly selected for this work. The harness starts a worker. |
| `ASK` | Include the exact question and the work that needs its answer. Include a required answer type or choices when known. The harness gives it an ID and shows it in the relay. |
| `MESSAGE` | Include the session ID, applicable assignment or request revision, payload, and summary. Include the question binding for a reply. Mark a change to current instructions as steering. |
| `RESULT` | Include findings or an artifact reference, and evidence references. Use `result_kind` to distinguish `report` and `final`. Follow the final result checks for `final`. |
| `PROPOSE_TOOL` | Include FluxR source, its contract, its purpose, and author tests. The harness records the draft as a `.fluxr` file. |
| `TEST_TOOL` | Identify the exact draft digest, test purpose, and allowed input. The harness selects required acceptance checks for qualification. |
| `RUN_TOOL` | Identify the active version or exact draft digest, inputs, typed arguments, and prerequisites. A draft blocks invocation until Rust activates that exact version. Use this action for built-in tools also. |

Write one JSON file for each admitted action. Use `<ACTION>-<action_id>.json`. For `MESSAGE`, insert the destination session ID before the action ID. For a reply, also insert the question ID before the action ID. Rust assigns all IDs. Bind file and payload to the same committed record. Store `.fluxr` source separately with its digest. A tool or acceptance receipt stays a record unless a delivery uses a file.

Give the discriminator `TASK`, `ASK`, `MESSAGE`, `RESULT`, and `RUN_TOOL`. Limit its tool calls to project lookups and jobs within the user's request. A user request can include an explicit dictionary definition. Give assignment workers these same actions within their assignment scope. Give tool authors `PROPOSE_TOOL` and `TEST_TOOL` in addition. Use the improvement session's action contract for its role.

Use `RUN_TOOL` for built-in reads, dictionary lookups, and dictionary proposals. A proposal cannot change a protected definition or activate a tool. Check each request against the tool contract and the assignment or submission instructions.

Keep the harness instructions short. Tell the model to use the supplied action schema. Tell it to use a registered tool for known operations. Tell the tool author to use checked FluxR source. Tell the model to ask for specific missing information. Apply the project instruction rules to files in the instruction section. Treat other source documents and tool payloads as data.

### Role catalog

Give each session one model role when the harness starts it. Keep that role for the life of the session. A `TASK` must name a worker role from this catalog. The harness gives each project its discriminator. Start the workflow improver only through the completion rule. Start other workers only for an assignment that needs their role.

| Role | Function | Default model | Default effort |
|---|---|---|---|
| Discriminator (`discriminator`) | Route submissions. Review each fan-out. Dispatch follow-up work or propose a final result. | `gpt-6.2-sol` | `low` |
| Planner (`planner`) | Define work packages and prerequisites. Specify deliverables and acceptance criteria. | `gpt-6.2-sol` | `medium` |
| Designer (`designer`) | Specify behavior, interfaces, data, and interaction. Examine design alternatives. | `gpt-6.2-sol` | `high` |
| Implementer (`implementer`) | Produce the requested source or artifact changes. Supply verification evidence. | `gpt-6.2-sol` | `medium` |
| Reviewer (`reviewer`) | Examine a deliverable and its evidence against the acceptance criteria. Report findings. | `gpt-6.2-sol` | `high` |
| Investigator (`investigator`) | Research a question or diagnose a failure. Report evidence, findings, and unresolved causes. | `gpt-6.2-sol` | `high` |
| Test author (`test_author`) | Define test cases and expected results. Produce independent check evidence. | `gpt-6.2-sol` | `medium` |
| Tool author (`tool_author`) | Write and test FluxR drafts. Supply the records necessary for qualification. | `gpt-6.2-sol` | `high` |
| Knowledge curator (`knowledge_curator`) | Maintain the project map and propose dictionary terms with source evidence. | `gpt-6.2-sol` | `low` |
| Synthesizer (`synthesizer`) | Combine selected results and evidence as an assigned deliverable. Report unresolved differences and holds. | `gpt-6.2-sol` | `medium` |
| Workflow improver (`workflow_improver`) | Examine completed requests. Create, repair, and test reusable tools. Publish verified improvements to project workflows. | `gpt-6.2-sol` | `medium` |

A model role gives no authority to accept work, activate tools, or commit effects. Use the verifier, registry, and effect rules for those operations. Use Rust for job scheduling, read selection, cache decisions, protocol validation, and tool qualification. These operations have no model preset.

### Model settings and turn seal

Let the user change the global default model preset for each role. Let the user save or remove a project override for each role at any time. Treat the model and reasoning effort as one pair. Send the effort as the OpenAI reasoning setting. Check the provider capability data for each pair. Use models that support the action schema and the required input types. [OpenAI model catalog](https://developers.openai.com/api/docs/models) · [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)

At each turn seal, use the latest committed project override for the role if one exists. Otherwise, use the latest committed global default for that role. Order a concurrent settings save and turn seal in the store. Apply this rule to the discriminator and to every worker. An unsealed turn uses the current settings when it reaches the seal. A session that is waiting keeps its wait condition.

Keep the preset and input of a sealed turn for every attempt of that turn. A settings change does not cancel a sealed turn. A retry uses the same preset and input with a new attempt ID. On a user request, close an unaccepted turn and seal a replacement with a new job ID. Apply the current settings to the replacement. A late outcome from a replaced turn cannot deliver actions.

Reject a known invalid model and effort pair when the user saves it. If the provider or inputs make the saved pair unusable, show an explicit error for the affected turn. Do not substitute a different preset. Keep unrelated work eligible.

## Routing and scheduling

### Routing of user instructions

Give the project discriminator the submission's reply target, term references, and project constraints. Supply a limited list of candidate sessions and tools. The discriminator can request a scoped lookup. Status reads and typed controls such as **Run again** and **Stop** go directly to the harness.

Link each proposed disposition to original prompt byte ranges or submitted attachment references. Record each actionable span and its disposition under a stable ID. Context spans can overlap. Dispatch an admitted disposition once. Hold unresolved spans against their dependent requests. If the request is unknown, keep the question at the submission level. Show unresolved spans in the relay. Ask for the intended work if a submission gives no clear instruction.

Apply each shared constraint to all the actions it governs. For the input “Prepare A and B. Publish B after I approve it,” both preparation actions can proceed. Hold the publication action until approval. Dispatch all clear independent actions.

Give a worker action the authority of its source assignment. Derive user authority only from an authenticated user submission or typed control. For model-proposed steering, check the exact user instruction and its destination before changing revisions. A worker message can supply information within its scope. It cannot claim to change user instructions or route into a different project.

### Questions and replies

Bind a worker question to `(question_id, question_revision, session_id, assignment_revision)`. Bind a discriminator question to the current request revision or to the original submission ID. The **Reply** control supplies this binding to the discriminator. The discriminator puts the exact answer span or specific answer attachment references in a `MESSAGE`. Address a discriminator answer to the same question scope. Give other instructions in the reply submission their own dispositions.

If a reply has a clear target but an unclear answer, ask for the missing information. If “yes” could answer two open questions, show the two choices. Hold only the affected work and its dependents.

Validate the question binding, answer source, and applicable revision when the answer transaction commits. A clarification can fill an unset parameter in the current revision. Steering makes a new revision for the affected request or assignment. For combined steering and reply, bind the answer to the new revision only if the same question still applies. Otherwise ask again. Commit at most one current answer to a question revision. Treat an identical retry as the same answer. Show a conflict for a different answer. Do not restart closed work because a reply arrives.

Commit the answer, the question's `answer_queued` state, and the inbox entry together. Select the next turn's inbox items before dispatch. Use only deliveries for its submission, request, or assignment. Pin `included_in_turn` items to that turn until the harness accepts an outcome or abandons the turn. Show `queued`, `included_in_turn`, and `consumed` as separate states.

Consume inbox items only when the harness commits a valid turn outcome. Mark the question answered when its answer is consumed. An item that arrives during an active call is eligible for a subsequent turn. Preserve the same selection for a retry. If the harness abandons the turn, return still-valid items to the queue. Record unavailable or cancelled items separately. An unread item prevents normal closure. Treat a change to a consumed answer as new steering.

### Work conditions and acceptance

Use one scheduler for model turns, tool tests, and tool invocations. A prerequisite identifies an exact answer, artifact, or acceptance receipt. Examine the condition when a wait starts and when related state changes. Reject an explicit dependency cycle. If no work can proceed, show the holds and their causes.

Record each criterion, expected result, and verifier when the harness admits the work. Bind each acceptance receipt to the exact evidence and current work revisions. Use a registered deterministic check or explicit human acceptance. A worker's `RESULT` supplies evidence. It cannot supply an acceptance receipt for its own result. A typed acceptance identifies the criterion, evidence, and revision the user saw.

Each worker response contains a `next` field. Use `wait_for` to name an inbox item, a job receipt, or another prerequisite. If the condition is true, the session is ready. Give each wait its own ID. Use one wake for each wait and event pair. A shared receipt can wake several distinct waits. Use `complete_when` to reference the assignment's acceptance criteria. Check the full set in the current assignment record. Finish the assignment when all required receipts exist and its own work and inbox are clear. This change needs no new worker turn. Track its descendants and request separately.

Send a tool receipt to a worker when it needs the receipt to continue. A failed draft test can make its author ready with the diagnostic details. A successful tool invocation with a direct result does not need a new model turn.

Validate the request and assignment revisions before delivery, result acceptance, or release of dependent work. On request steering, mark affected descendants superseded or bind them to the new request revision. Retain unaffected evidence only after a verifier confirms its applicability. Show late results from a superseded revision. Such a result cannot release work for the new revision.

### Fan-out review

When the harness admits `TASK` actions, group them by request and parent assignment. Give each group a fan-out ID. Record the request revision. Use an empty parent ID for a request's first-level assignments. Freeze the child list at commit. Attach all descendants to the same request. A single child uses the same review rule.

A fan-out reaches a review point when its branches have finished or have a recorded wait. Make sure no branch has ready or running work. Include accepted, failed, cancelled, and waiting outcomes. Process ready descendant fan-out reviews before ancestor reviews. A pending descendant review holds an ancestor review until its decision commits. A committed wait can pass that boundary. Only an acceptance receipt satisfies a criterion.

Commit a review delivery with the state change that makes the fan-out ready. Use one delivery for each fan-out, request revision, and distinct set of branch outcomes and waits. Recheck readiness before the turn seal. Seal the request revision and the exact fan-out state with the turn. Use the existing project discriminator session and its current model preset. Include the request's current intent and criteria, branch outcomes, evidence references, and remaining work. Include exact IDs for open questions, outstanding effects, and prerequisites. Use recorded references when full evidence does not fit. Do not load the project timeline.

In the review response, require `review_id`, `achieved`, and `remaining`. In `achieved`, give criterion IDs and evidence references. In `remaining`, give criterion or wait IDs and reasons. Validate these references against the sealed review input and current records. Check the request revision and fan-out state before admitting actions. The review can make one of these decisions.

| Decision | Actions and state change |
|---|---|
| `continue` | Emit one or more `TASK` actions. `ASK` can hold work that needs an answer. Record the new fan-out under the same request and parent scope. Link it to this review. |
| `wait` | Record one or more conditions that must change before another review. Use `ASK` if a new answer can release work. |
| `settle` | Record that this fan-out needs no more work. Use when the request still needs other work or review. Release its ancestor review when ready. |
| `finish` | Emit exactly one `RESULT` action of kind `final`. Give the result and evidence for the current request. Offer this decision only for a first-level review that can pass the final checks. |

Start a new review after a recorded condition changes and the fan-out reaches its next review point. Deduplicate by fan-out ID, request revision, and branch outcome state. A committed review decision ends its pending state. Only its unresolved work or waits remain. An idle timer does not cause a new review. Reuse the same review delivery after restart. If all final checks become eligible later, queue one request closure review for the exact set of current prerequisites. Use the most recent first-level review as its source. Reuse a pending first-level review that can check the same set. Offer `finish` only when the final checks can pass.

Give each follow-up task an unmet criterion or a reason that depends on current evidence. A replacement for a failed branch identifies that branch and keeps its acceptance criteria. Preserve the failed outcome in the records. Only accepted replacement work can satisfy those criteria. Require user steering to remove or change a required criterion. Require a new user instruction before replacing a branch the user cancelled.

### Final result and request closure

Require a discriminator proposal to close a request that has a fan-out. Show a worker `RESULT` of kind `report` as assignment evidence. For a request without a fan-out, the discriminator can give a final result when the acceptance checks pass. For a routed tool request with no fan-out, Rust can make the final `RESULT` from the successful tool receipt. Apply the same final checks. If its tool fails and no retry remains, show the failure and mark the request failed. A typed invocation outside a routed request returns only its tool receipt.

Use a direct tool as the closing action only if its validated receipt can satisfy every acceptance criterion. If the work needs interpretation of the receipt, use an assignment for that work.

Check the current request revision before admitting a final result. After a fan-out, also check the review's fan-out state. Make sure all required criteria have current acceptance receipts. Make sure all required branches and their descendants are accounted for. Check that no other review, work, effect, question, or unread delivery for this request remains open. Include the current review and delivery consumption in the prospective commit state. A failed or cancelled branch needs accepted replacement work or an explicit user change to the request.

If a queued user submission explicitly targets the request, defer the final result until routing commits. Apply any steering as a new request revision. Recheck the deferred result after routing. If its review became stale, use current records for the next review. Reject other stale review actions against their affected scopes. Unrelated project submissions do not stop closure. A later submission about closed work can start a linked request.

Commit one final `RESULT` record, the completed request state, a pending relay event, and the improvement session together. Publish the final file with the normal continuation recovery rules. After verification, assign the event cursor and release the final result and improvement session in one transaction. Later events can publish while a final file is pending. Reject more than one accepted final result for the same request. Keep the project relay ready for new submissions.

If access changes before final publication, mark the result unavailable and publish that status under the same event ID. Keep the request's final state. Release the improvement session with only the evidence that it can still access. If the user cancels that session, record the cancellation.

If a final check fails, keep the request open and show the reason. Wait for a required event or use the bounded correction rules for invalid output. When the user stops a request, record its cancellation separately. Keep any effect in progress visible until its outcome is known.

### Limits and recovery

Allow one active model turn per session. Set limits for input bytes and tokens, reasoning and output tokens, depth, simultaneous calls, and attempts. Limit the assignments in each fan-out. Limit the total assignments and reviews in each request. Reserve capacity before dispatch. Reserve some model capacity for the discriminator. Show the reason when a limit stops work.

Record the attempt before the harness sends the request. If the transport fails after possible provider acceptance, record an uncertain outcome. Account for that attempt. Give each retry a new attempt ID.

Replay uses committed records. It does not call OpenAI. Accept at most one final outcome for each job. Local deduplication cannot establish whether the provider processed or billed an uncertain attempt.

### Delivery summaries

Show a short ASD-STE100 summary for each session delivery. Keep the exact payload available. Use **Read the prompt.** or **Read the prompt and its attachments.** for the first discriminator delivery. Use **Read the attachments.** for an empty prompt. Show this summary before the model reads the submission.

Use **Review the fan-out.** for a fan-out review delivery.

Use **Improve the workflow.** for the initial improvement session delivery.

Use reviewed templates for harness events and typed tool results. Use canonical project technical terms when applicable. Require a summary for model deliveries. Use one instruction per sentence. Keep restrictions that change the action.

Use a string or `null` for the model's summary field. Treat `null` as a missing summary for that action. Validate sentence length and vocabulary with the permitted technical terms. Check identifiers and structured values that the harness can compare with the payload. Keep model summaries labeled. These checks cannot establish full language conformance or semantic equivalence.

A missing or rejected summary holds the affected delivery and its dependents. Commit valid independent actions. Send a diagnostic receipt to the source session for a limited correction attempt. Keep the original action ID. If the limit is reached, show the hold in the relay.

## Workflow improvement

### Trigger and execution

Create one improvement session for each completed request. Key it by project ID and request ID. Commit its record and initial delivery in the same transaction that records the final result. A final tool receipt for a routed request uses this rule also. Start the session after the relay can show the final result or its unavailable status. Recover the same pending session and its job state after restart. A retry keeps the same session ID.

Keep the completed request closed while the improvement session runs. Use a separate state, scope, and budget for the improvement session. An improvement report, cancellation, or failure cannot start an improvement session. A typed tool invocation or **Run again** creates no request and starts no improvement session.

Run one improvement session at a time in each project. Queue the remaining sessions in completion order. Give foreground work priority. Reserve bounded capacity so a busy project can still run queued sessions. Limit model calls, tokens, tool jobs, and elapsed time. Use bounded tool jobs in the same FluxR VM. End the session when it needs evidence that it cannot obtain within its budget. Keep the candidate for future work. A report closes the session after its own jobs and deliveries have a recorded outcome. Mark unfinished proposals deferred before closure.

### Evidence and decisions

Give the session the closed request's acceptance results, executed steps, and exact evidence references. Include a compact record of retries, blocked steps, tool failures, repeated reads, and manual work. Give it counts for model calls, tokens, file I/O, cache use, and elapsed time. Separate measurements for foreground work and improvement work. Select related project records and tool versions by task or tool identity. Use the normal context and retention rules. Keep reusable fixtures and published records within their source read scopes. Test on an independent fixture or a verified small case when the original evidence has narrower scope.

Check for an existing tool, candidate, or workflow record before writing another. Prefer reuse or a correction to a known tool. Extract a new tool when the operation has stable typed inputs, outputs, and a useful test. Record recurring context gaps as proposals for the project map or dictionary. Preserve human-edited wording and the authority of project instruction files. Record proposals for changes to routing defaults, repository files, or external services for future user work.

Use `RUN_TOOL`, `PROPOSE_TOOL`, `TEST_TOOL`, and `RESULT` of kind `report`. Keep operations within project evidence, draft tests, registry proposals, and project knowledge records. Use the existing rules to update those records. Keep the final result and its artifacts fixed. Give the report an outcome and exact improvement record references. Use `published`, `deferred`, or `no_change` for the outcome. For `published`, include committed publication receipts. For `deferred`, record what the candidate needs. The scheduler records failure or cancellation separately.

Record one improvement candidate for each equivalent operation or defect in the project. Use a unique key with its operation ID, input contract, affected version or workflow ID, and source scope. Record the evidence, intended change, and required checks. Attach each request's evidence once. Reuse an existing test receipt only when its version, input, case, and qualification rules still match. Recheck source access and registry versions in the publication transaction.

Use a workflow record to identify when an existing tool can replace repeated work. Record the applicable input types, preconditions, exact tool version, and verification receipts. Before using the record for automatic selection, check that the tool is available and its contract covers the operation. Verify the result on an independent case. Use a qualified FluxR tool if the replacement needs several new computation steps.

### Qualification and publication

Use the normal isolated tests and independent qualification for each exact tool version. Include a fixture for the observed error or repeated operation. For a tool repair, reproduce the error on the old version. Make sure the new version gives the expected output and passes the existing valid cases. Use the exact old version and comparable inputs for cost comparisons. If evidence is insufficient, keep the draft and record the missing check.

After qualification, let the harness activate a new tool and select its first default version. For a repair, use `tool.promote` with the candidate version, expected current default, and evidence references. Permit automatic promotion when existing callers can use it under the same public contract and supported inputs. Require independent regression evidence and passing contract checks. Require the same scope and no additional effects or host services. Check the expected default in the promotion transaction. If it changed, keep the evidence and examine the change before attempting promotion again.

For a change that needs a different contract or wider scope, record a migration proposal. Keep the old default until the user accepts the migration under the existing rules. Keep each accepted invocation bound to its recorded executable version. Key cached results by exact version. Record promotions and their evidence. Let the user restore a prior qualified default through the registry.

Publish active bindings and their project dictionary links in the same transaction. Record where a tool applies and the evidence that supports reuse. Supply relevant workflow records and tool references in future turn inputs. Select them by scope and task. Keep improvement evidence accessible through project lookups and apply the normal retention rules. A typed invocation of a published tool uses the normal deterministic execution path.

### Efficiency checks

For each change, record the baseline, expected benefit, test inputs, and observed result. Require correct outputs and preserved acceptance checks. Compare applicable counts of steps, model calls, tokens, disk reads, or VM fuel. For timing claims, use repeated comparable runs with the same cache conditions. Record estimates separately from measured results. Include the cost of the improvement session when measuring the project's net benefit. Use a verified gain or a reproduced defect as the reason to publish a change.

Measure later reuse with receipts that identify the workflow record or tool version. Compare equivalent work before and after the change. Count each improvement session's cost once. Record a gain only for the measured workload. Record regressions and flag the affected version or workflow for review. Keep measurements and evidence out of ordinary turn input unless relevant. Use current results to select the next candidate with the highest supported benefit.

## Project context and dictionary

### Project instructions

Look for `AGENTS.md` and `CLAUDE.md` at the project root when the user opens it. Match both names without regard to letter case. Include both files when both exist. Use their actual names in the turn record. Use live working files, including untracked and modified copies.

Use the root files for every role, including the discriminator. For a worker in a selected worktree, use the corresponding root in that worktree. Use the current working root for instructions when the work reads a historical snapshot as evidence.

For an assignment with recorded target paths, find instruction files in each applicable directory below its working root. Include the files from the root to each target directory. For a file target, use its parent directory. Include each instruction file once with its directory scope. Use the existing path containment rules for discovery.

Let the session discover more paths with read tools. Check the resolved targets of a model-directed effect before execution. If applicable instructions for a newly reached directory were missing, hold the write. Give the session the target path and held action ID. Load the instructions in the next turn. Require a replacement action that identifies the hold. When the replacement commits, mark the old action superseded and update its dependents in the same transaction. Check that the replacement satisfies the same requirements before updating a dependency.

Put the project instructions in the first section of `input.json` and the corresponding model input. Include each file's relative path, directory scope, digest, and full text. Use an empty section when no file applies. Put this section before the submission, assignment, skill instructions, attachments, source excerpts, and inbox items. Send the harness protocol instructions from `SYSTEM.txt` separately. Keep the original `PROMPT.txt` bytes unchanged.

Order instruction files by directory depth and then by relative directory path. Within a directory, put `AGENTS.md` before `CLAUDE.md`. This order makes the input reproducible. Apply instructions only within their directory scope. Give a more specific directory's instruction priority for that directory. Follow an explicit user instruction that resolves a project rule. If applicable files at the same scope conflict, ask about the affected work. Dispatch clear independent work.

Apply the protocol and tool contracts to every action. A project instruction file cannot grant a tool capability or authority for an effect. Treat CLI commands in these files as project information. Use registered tools for corresponding operations. A file reference in an instruction file uses the normal context selection rules. An attached or quoted copy of an instruction file is source data.

Refresh the file selection before each turn seal. Use the watched source and file cache rules for discovery and reads. Record each source version and the exact bytes with the ordered selection. Edits, additions, and removals apply to the next unsealed turn. A retry uses the sealed selection. Require readable UTF-8 text that fits the input limit. If two entries in one directory match the same file name, report a name collision. For either error, hold the affected turn and show the cause.

### Context selection

Make a small project map from selected snapshots and project sources. Use the inventory, search, and file-read tools to find components, verification commands, and stale links. Record existing commands as project information. Use only the corresponding registered tools to do verification. A session can propose a description of a component with its source references. Keep project decisions and their authority separate from definitions.

For each turn, select the assignment instructions, applicable project constraints, source excerpts, inbox items, and exact references. Include applicable instructions from skills that the user selected for the assignment. Record the input and its dictionary view. A worker can ask for an earlier result or decision. Unrelated growth in the relay timeline does not change an equivalent turn input.

### Dictionary types and scopes

Use three kinds of dictionary entry.

| Kind | Symbol | Lookup result |
|---|---|---|
| `knowledge` | `◇` | Definition, scope, and evidence for a concept, calculation, role, process, or project term. |
| `tool` | `⚙` | Description, scope, available tool contract, and an optional draft or exact executable version. |
| `skill` | `✦` | Definition, scope, installation source, and exact installed skill version. |

Each entry has one kind. Different entries can have the same name when the meanings differ. A name or kind gives no permission to execute a tool.

Use knowledge entries to explain people, decisions, policies, source files, data sets, artifacts, and workflows. Link these entries to their records. Those records keep their own authority. Give a callable workflow a tool entry.

Load the technical definitions from this document into the app's **RelAI** scope as knowledge entries. Make these definitions read-only. Use the document version as their source. Add tool entries for built-in capabilities from the registry. Project definitions do not change protocol meanings.

### Owned and linked dictionaries

Give each dictionary a stable ID and one owner project. Give each project an enabled default owned dictionary. The user can create, rename, archive, and edit owned dictionaries. The user can link dictionaries from other projects. Enable or disable each dictionary for the receiving project without changing its other links.

An enabled link gives read access to the shared definitions and their permitted term links. It gives no access to the owner's files, tool permissions, or installed skill use. Follow a term link only if the receiving project has access to that target. A link does not give access to a third dictionary. A tool or skill term can be readable while its target is unavailable in the receiving project.

The user can edit a linked dictionary by opening its owner scope. Show the owner and the other projects that have enabled it before the user saves. Make local wording changes in an owned entry that refers to the source term. A session cannot edit or propose changes to a linked dictionary from the receiving project.

Use the daemon's grant records for access checks. Record changes to a dictionary link with an increasing grant revision. Disable a link before supplying any further lookups from it. Unlinking removes the link from the project. Invalidate related suggestions and cached definitions on grant changes. Preserve already committed turn records under the retention rules. Check current access at lookup, delivery, and turn dispatch. If access changes after sealing, use the turn retirement rule.

### Entry creation and changes

Give each entry a stable ID in its dictionary and increasing revision numbers. Record the canonical name, aliases, scope, short description, full definition, source references, author, and state. A source reference identifies the exact document revision or user-message span.

Allow manual and automatic creation of knowledge and tool entries in the project scope. The harness creates a skill entry when the user installs a skill. A session can add an active knowledge entry from an accessible project source with an explicit definition. The harness makes sure the source exists. An entry with a conflicting meaning in the same scope prevents automatic activation. An inference from usage is a proposal. Show a conflict until it has a resolution. Combine proposals with the same scope, normalized name, and exact definition.

A human edit makes a new revision with the label **Human edited**. A direct user definition in the relay keeps the user's exact words and has the same protection. An ordinary use of a term is not a definition.

To change protected wording, an alias, or a scope, a session supplies a proposed change, evidence, and affected uses. The harness applies the change only after the user accepts it against the current revision. If the revision changes first, keep the proposal and show the conflict. A source change can make the definition stale. It does not rewrite the definition.

A tool entry may describe an unbound capability or refer to a draft. Activation makes an exact version callable. Registry operations update executable bindings by stable tool and entry IDs. A name match alone cannot replace an entry. Keep human-edited wording and aliases when the binding changes. A change to dictionary text cannot change the executable, schema, permissions, or qualification.

A lookup of an earlier revision gives that revision and its exact tool binding, if it has one. A lookup cannot start an invocation.

The user can edit a skill entry's definition and aliases under the same revision and protection rules. Dictionary edits cannot change its package binding or make a session use it.

### Autocomplete

Show suggestions from the enabled dictionaries as the user types. Put the selected term's short description **above the suggestions**. Show its type, source dictionary, and edit or draft status. Support keyboard selection and dismissal. Let the user open the full entry and edit it with the correct authority. Autocomplete uses no model call.

Show active knowledge, named tools, and installed skills in ordinary suggestions. Show knowledge proposals in dictionary search. Normalize search keys with canonical Unicode normalization and case folding. Keep the original text bytes. Use Unicode word boundaries for whole-term matches and grapheme boundaries for selections. [Unicode normalization](https://www.unicode.org/reports/tr15/) · [Unicode text segmentation](https://www.unicode.org/reports/tr29/)

A selected suggestion binds that occurrence to an exact revision. An explicit selection has priority over an automatic match. An automatic match needs one meaning across the enabled dictionaries in scope. For overlaps, select the longest whole-term match. If a name is ambiguous, leave it unbound. Show the type and source dictionary for each choice. Ask which meaning applies when it affects the work.

Skip URLs, inline code, and fenced code during automatic matching. Allow a user to attach a marker to a selected span. If an edit changes the selected span, cancel its binding. The user can select it again or let automatic matching apply. An older selection can still use its displayed revision while it is accessible. Do not replace it silently.

### Marker encoding

A symbol is metadata. The harness never reads a marker from characters in the user's text.

Record the original UTF-8 text and its digest. Record ordered, nonempty byte spans with an inclusive start and an exclusive end. Reject an overlap or an invalid range, kind, ID, revision, or scope. When the user edits the text, calculate new ranges against the new bytes.

The UI shows each marked span as a colored, clickable term. It does not show the generated marker character. Plain-text copy gives the original characters. The model receives ordered JSON parts. Each part contains its exact text and, for a marked part, its term metadata. Joining the text fields gives the original bytes.

The example represents `Compare (billing cutoff) with billing audit. See monthly close.`

```json
{
  "parts": [
    {"text":"Compare ("},
    {"text":"billing cutoff","term":{"kind":"knowledge","mark":"◇","dictionary_id":"d01","id":"k17","revision":3}},
    {"text":") with "},
    {"text":"billing audit","term":{"kind":"tool","mark":"⚙","dictionary_id":"d01","id":"t05","revision":2}},
    {"text":". See "},
    {"text":"monthly close","term":{"kind":"skill","mark":"✦","dictionary_id":"d02","id":"s09","revision":1}},
    {"text":"."}
  ]
}
```

Spaces, punctuation, and literal `◇`, `⚙`, or `✦` characters do not change these boundaries. Each repeated or adjacent occurrence has its own span. The literal symbol in `billing cutoff◇` is plain text. Automatic matching can identify the name independently.

A forwarded span keeps its dictionary ID, term ID, and revision. New text uses the current enabled dictionaries. Include short references in model input. An edit to a dictionary cannot change input that the harness already dispatched.

### Term links and model expansion

A definition can contain links to other terms. Store each link with the term span, target dictionary ID, and stable term ID. Resolve the target revision in the selected dictionary view. A definition can link to itself. Two or more definitions can link to each other.

Open one definition for each click. Keep navigation history for **Back**. A link keeps its target when a definition changes names. When the target is unavailable, show the term and its unavailable state. Do not substitute a different definition.

Use `dictionary.expand` when a session needs definitions for a list of terms. Accept an ordered list of exact term references in the requesting turn's dictionary view. Check access to each entry when the tool runs and when the receipt goes to the session. Return each requested revision once, in order of first request. Include the definition, its type, source dictionary, and exact links. A revoked, unavailable, or invalid reference has a separate outcome. Do not expand links automatically.

Reserve space in the next turn for the complete receipt. Reject a batch that exceeds the count or byte limits. Give the session the limits so it can request a smaller batch. Include every available definition from an admitted batch. Put the complete receipt in one addressed inbox delivery. The scheduler gives it to the requesting session in its next eligible turn for that work. That turn waits for the receipt. Apply the normal delivery and consumption rules.

If the discriminator requests expansion, hold only the affected parts of that submission. Other submissions can proceed. A follow-up turn for the held parts keeps the original submission and recorded dispositions. Use those dispositions to prevent repeated dispatch.

## Skill installation and use

### Package and sources

Give the user an **Install skill** control. Accept a local directory, a ZIP package, or a directory in an approved HTTPS Git repository. Resolve a Git source to an exact commit before reading the package. Use the in-process Git and archive services.

Require `SKILL.md` with a name, short description, and instruction body. Accept references and assets from the same package. Use the Agent Skills format for `SKILL.md`. Validate the optional `relai-skill.json` manifest separately. Its `tool_refs` records have `tool_id`, `version`, and `required` fields. Its `tool_drafts` records have `source_path`, `contract_path`, and `test_paths` fields. All paths are relative to the package. [Agent Skills specification](https://agentskills.io/specification)

Validate package paths, size limits, file types, and declared dependencies. Reject a path that escapes the package. Report unavailable tool links and incompatible requirements before installation. The user can install such a package for inspection. Hold use if a required tool or dependency is unavailable or incompatible. At assignment binding, record the exact versions of required tool links and dependencies. A dependency cannot load another skill's instructions. Each skill whose instructions the assignment needs requires explicit selection. Imported instructions and `allowed-tools` give no additional authority. Treat files in other scripting languages as inert package data.

Record the package digest, source, manifest, and exact file contents. Commit the skill version and its dictionary entry together. Use the package description as the initial definition. A new installation of the same skill makes a new version. Preserve protected wording and aliases when the package binding changes.

Give each bundled FluxR draft its own identity in the tool registry. Apply the same testing, qualification, and activation rules as for a worker draft. Installation supplies drafts. It does not execute them or make them callable.

### Use in an assignment

Load skill instructions only for an assignment that the user explicitly selects. The user can choose **Use skill** for that assignment or give an explicit instruction in the relay. The discriminator uses the same routing rules for a natural-language instruction to use a skill. Adding a skill to an active assignment is steering.

Bind the assignment to an exact installed skill version. A `✦` mention only identifies a definition. Selecting an autocomplete suggestion also only sets the reference. A worker can recommend a skill through the relay. It cannot extend the user's selection to another assignment.

Include the instruction body only in turns for the selected work. Use `skill.read` to request exact permitted references and assets when needed. Do not send the full skill catalog to a model. Hold work if a required instruction does not fit the turn limit. The assignment and project policy control which actions can run. If selected skills give incompatible instructions, ask about the affected work.

Give the user controls to inspect, update, and remove installed skills. An update makes a new version and does not change a pinned assignment. Removal stops new use and blocks future reads of the removed version. Recheck access when a turn that used the skill commits its actions. Pause affected work that still needs the skill. Keep existing turn records under the retention rules. A tool qualified from the package keeps its separate registry state.

## FluxR tools

### Daemon and VM

Link the FluxR Rust source admission and portable VM code into the daemon. Use the checked source parser and runtime in process. The project compiler's CLI and Python frontend are outside the invocation path. Pin the FluxR revision and runtime configuration. Use one VM for draft tests, qualification, and every tool invocation. A built-in tool has a checked FluxR entry point that calls its Rust host service. Disable native execution tiers for tool code. [FluxR runtime guide](https://github.com/Ryd-rb/FluxR-lang/blob/main/docs/unified-runtime-v1.md)

Allow one active tool job in the VM at a time. The scheduler can continue to route work and make concurrent model calls while tool jobs wait. Keep model credentials in the daemon's OpenAI adapter. Tool code cannot read those credentials.

A session can use `RUN_TOOL`. A tool author or workflow improver can also use `PROPOSE_TOOL` and `TEST_TOOL`. There is no shell, general evaluator, or direct script execution API for sessions. Accept only checked `fluxr 1` source for authored tools. Reject native Rust declarations, external dependencies, and undeclared or unauthorized imports. The daemon and its tool services never start child processes. This includes Git commands, shell commands, and FluxR CLI commands.

### Host access

Run the daemon under the Windows account that starts it. Use that account's full access without elevation. Use Windows access checks as the host access policy. A typed filesystem request can name a host path outside the registered project roots. Project root registration selects the sources for project indexing and watches.

Keep routing, automatic context, retention, and receipts in their project scopes. A typed host path does not make the path part of a project index. A file on the computer enters model input only through an attachment or a recorded source selection. Use typed record APIs for the daemon's private data. Exclude private storage and aliases to it from generic filesystem tools. The daemon holds its credentials in its adapters.

Apply the tool contract and the user's instructions to exact operation targets. Use the existing effect rules for file changes, Git writes, erasure, and external operations. The host access policy makes Windows resources available to the daemon. It gives a model no direct operating system interface.

### Call isolation and limits

Give each tool job fresh VM call state, limits, and a new table of input handles. Bind the declared host services and inputs in Rust before the call. Each host service validates its arguments and operation rules. A handle from another job gives no access. A host service cannot enter the VM again or start a new tool job. Clear call state and handles when the call ends. During testing, only the test job can call the draft's VM module.

Give tool code typed arguments and handles for snapshots, live paths, or artifacts. Resolve handles in Rust. At the FluxR service interface, use supported scalar and list values or opaque handles. Keep record layout and ownership in the Rust adapter. Tool output is data. It cannot issue a relay action. For a change, the VM returns a typed effect request. A guest callback cannot commit a live write or external action. Rust checks and executes effects after the VM call. Bound scratch data to the tool job.

Bound source size and compilation work. Set VM limits for fuel, cells, and frames. Limit Rust buffers, host service calls, and output separately. VM fuel cannot interrupt a Rust callback. Use callbacks only for bounded work with an enforced deadline. Do work that can block for longer in a separate in-process job phase. Resume with typed input in a new VM call. Give the whole job a receipt only after its required phases finish. Record cancellation immediately. Wait for the active VM call to end before starting another. Discard output from a cancelled call. [FluxR current status](https://github.com/Ryd-rb/FluxR-lang/blob/main/docs/current-status.md)

### Git and filesystem services

Use `gix` from gitoxide for Git objects, refs, history, worktree operations, and fetch. Use its component crates and in-process Rust adapters for operations that need more than a public `gix` call. Use an in-process Rust Smart HTTPS send-pack adapter for `git.push` if needed. Use `gix` objects and pack data in that adapter. Use only approved HTTPS remotes. Keep credentials in the in-process adapter. Disable Git hooks, external filters, credential helpers, and executable fallbacks. Verify the exact pinned implementation of each operation on Windows. [gix API](https://docs.rs/gix/latest/gix/) · [Gitoxide capability status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md)

Use Rust standard-library APIs for filesystem and text operations. Use `std::fs::read_dir` for directory lists. Use `std::fs::read` or `std::io::Read` for file data. Use `std::fs::remove_file` for a file removal. Resolve live paths and check the actual targets. Enforce containment for relative paths, archive entries, and staging paths. Reject an unexpected link traversal. Keep committed artifact bytes fixed until authorized erasure. [Rust filesystem APIs](https://doc.rust-lang.org/std/fs/)

### Writes and recovery

Use `patch.apply` for approved changes to a live checkout. Require the checkout ID, expected Git revision, staged patch ID, and expected digest for each affected file. Require user authority for the exact write scope. Do not use staging tools on the live checkout.

Use the explicit host operations for ordinary live files outside a registered checkout. Require exact paths and expected state for each change. Check both endpoints of a copy or move. Use a staged artifact for new file bytes. Require explicit user intent for destructive changes. Resolve the actual target before classification. Use the checkout write rules if a target is in a registered checkout.

Before a write, record the exact targets, expected state, intended state, and applicable work revisions. Resolve actual file identities and check both paths of a move or copy. Recheck authority and state at the point of effect. Use held handles, ref locks, or an equivalent system operation to protect the check and write. Reject the write before modification if these checks cannot be protected. Use atomic publication for each individual write. For a move that cannot publish atomically, use a recorded copy and removal sequence with checks on both paths. Record which steps completed. A multi-file change can stop partway through.

On restart, reconcile each effect step with the recorded old and intended states. A match to the intended state records a completed step. Retry a step in the old state only if its current authority and expected state still hold. If neither state matches, record a conflict and preserve the external change. Hold the retry when the effect remains uncertain.

Stage only specified file versions. Make sure the index and branch head match the expected state. Build the commit tree from the parent and selected files. Keep unrelated staged changes. Change refs with an expected old object ID. A worktree removal needs user authority. Make sure the harness owns the worktree, its work is merged, and it is clean. Keep durable evidence. Push only to the approved branch. Send its expected old object ID with the update. Reject a non-fast-forward update.

### Connectors and schedules

Register connectors with typed schemas, permitted destinations, deadlines, and limits. Keep credentials in the daemon. Make sure the caller has authority for the exact operation before an external write. Record the operation and its expected state before sending it. Use the same service-supported idempotency key for all attempts of that operation. Without that key, retry only when the service can confirm that the earlier attempt cannot commit. A state observation alone can leave the outcome uncertain. Report an uncertain outcome and hold the retry. A connector cannot send commands or program code for execution on another host.

Use daemon timers for schedules and source events for monitors. Record the user authorization, time zone, condition, exact action, limits, and cancellation state. Identify an occurrence by schedule revision and planned instant or source event ID. Record the occurrence and its job together before dispatch. Recover due occurrences after restart. Ignore duplicate occurrences. If an earlier occurrence of the same schedule is running, record the missed occurrence. Recheck current permission before each dispatch. Cancellation stops new occurrences.

### Draft tests and qualification

A tool author or workflow improver writes an immutable FluxR draft and requests tests with `TEST_TOOL`. The harness binds the test to the exact draft and inputs. It compares the VM output with the expected output. It gives the session a receipt with the test case and observed difference.

A test can use a bounded, immutable task input under the same isolation rules. Bind test host calls to immutable fixtures or simulated effects. Keep live host paths and live effects out of draft tests. Its output is provisional. A provisional result cannot satisfy a prerequisite that requires an active tool. A change to the source, contract, runtime, or bound host services needs a new qualification.

For qualification, examine schema conformance, VM limits, and repeated output with fresh call state. Require acceptance evidence independent of the implementation. Use a trusted project fixture, a user-specified expected example, or a host check specific to the capability. Author tests help the author correct the tool. They do not suffice for qualification.

If the evidence is missing, keep the draft and its test receipts. The worker can ask for a specific example. When the required checks pass, the harness may activate the exact version within the project's allowed scope. This decision uses the recorded checks and requires no model call.

Record the source, schemas, FluxR revision, runtime configuration, host service versions, permissions, and test evidence digests. Record the portable execution route in the test and invocation receipts. Select the first default in the activation transaction only if a default is absent. Other active versions are callable by exact ID. Change a default through an explicit registry operation with an expected current version. A default must refer to an active version. Clear its binding when the version becomes unavailable.

Retirement blocks new invocations of a version. Already admitted invocations can finish while qualification and authority remain valid. Revocation stops new admissions and blocks uncommitted output and effects from that version. Retain the exact receipt for an effect that already occurred. Invalidate cached results for a revoked version. Check qualification revisions at admission, before an effect, and at delivery.

### Execution and reuse

Resolve one exact tool version. Pin it when the harness admits the invocation. Make sure it is active and the scope permits its use. Make sure the arguments and selected source meet its contract. Examine the remaining invocation budget. If a check fails, report the reason before execution.

If an invocation waits for a draft, wait for that exact draft's activation. Pin its active version when the wait clears. Another version cannot satisfy the prerequisite. If the draft becomes unavailable, return a blocked outcome with the cause. Validate the output before using it.

A typed invocation makes no model call on success or failure. A natural-language request goes through the discriminator. A valid `RUN_TOOL` can then finish and show its result directly.

Give a new invocation its own ID. Record its inputs, executable digest, and the source identity. **Run again** selects the exact version, arguments, and input source. It can use a new snapshot with the same executable.

Select a registered tool before authoring a new one when its contract covers the work. Use a parameterized tool or composition when applicable. If a repeatable operation needs a new capability, record the gap. A worker can author it within its limits.

## Shared execution and caches

### Eligibility and delivery

The daemon owns one single-flight index and a shared result cache. Select reuse rules when the tool qualifies. The rules identify the inputs and source dependencies that can change its result. The harness validates these rules against the tool contract and its host services. A tool cannot grant itself wider reuse.

Reuse pure results from exact immutable inputs. Reuse live read results only when all source dependencies have a valid generation. Record dependencies from actual host reads, including resolved aliases, absence checks, and traversed directories. Use shared execution only when the key covers every possible dependency for those arguments. If a dependency falls outside that key, discard the shared result. Check each caller's sources before running separately. Use neither cache nor single-flight for operations with effects or untracked external inputs. Cache only successful, complete results.

Use a key with the exact tool version, canonical typed arguments, and source identities. Include the project, effective read scope, policy revision, and limits that change the result. Include the required freshness for a live read. Include the generations of the declared paths or roots. Reuse a result only while every recorded dependency still matches. Start a new job when the keys differ.

After admission, look in the result cache before scheduling a VM job. On a miss, look for an equivalent queued or active job. Attach the call if an eligible job exists. Otherwise create one job. A request for fresh data can join only a queued job with the same freshness requirement. Record a separate invocation ID and delivery for each caller. Use the same job receipt for all linked invocations. A cache hit needs no VM or model call.

Check each caller's permission and scope revision again before delivery. Show whether the result came from a new job, a shared job, or the cache. A caller's cancellation or deadline removes only that caller. The harness can cancel the job when no valid caller remains. Keep cache entries and pending work bounded. Clear memory caches on daemon restart. Keep committed receipts and artifacts.

### Watchers and invalidation

Start recursive Windows directory notifications for each registered local root before the baseline scan. Watch the root's parent for removal, replacement, or rename of the root. Do not read unrelated files in the parent. Use the Windows notification API through an in-process Rust adapter. Keep the watcher in the daemon. [Windows directory notifications](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw)

Process file writes, creation, deletion, replacement, renames, and applicable metadata changes. Invalidate the file bytes, line indexes, and tool results for the changed path and file identity. A rename invalidates both paths and the related directory listings. Invalidate dependent subtree results when directory contents change. Treat a recorded absence as a dependency of its parent directory. Mark affected dependencies dirty before the daemon starts a live write. Reconcile their state before reuse.

On overflow, a lost watch, or an event with insufficient path information, mark the affected scope dirty. Give it a new generation. Stop cache hits for that scope. Reopen watches and rescan the affected directory before the cache can serve it again. Events that arrive during the rescan keep the affected paths dirty until the next reconciliation.

Do bounded periodic reconciliation even when notifications look healthy. Use file identity and metadata for routine checks. A required fresh read bypasses both live caches. Read the requested bytes or list the requested directory again. If the request requires content verification, compare bytes or a digest. Routine metadata checks alone cannot detect all same-size writes. A watched result identifies its observed generation. Use an immutable snapshot when a check needs a fixed view of several files.

Record the generations before a live read. Validate them when caching or delivering the result. If an event changes a dependency during the job, discard the old result. Retry within the budget or report that the source changed. A late result cannot fill the cache after invalidation. Report absence, denied access, changed input, and watch uncertainty as different outcomes. Serialize invalidation and result admission in the daemon. Do not change an immutable snapshot when its live source changes.

### File reads and overlapping ranges

For a normal file read, consult the shared memory LRU first. Include the project and effective read scope in every key. Key immutable bytes by digest. Key live bytes by watched source, file identity, and generation. Bypass the live cache if the source has no valid watch generation. A small file can use one entry. Use fixed byte blocks for a larger file. Read only missing blocks. Reuse cached blocks when requests overlap. Use the same cache for all tools that read file data.

Keep line offsets for the same file version. Convert line requests to byte ranges with that index. Give the data and index separate size limits. Count buffers that reads still use against the limit. Reserve memory for new reads before starting I/O. Remove the least recently used entries when a limit requires it. Evict invalidated data immediately. A normal hit on a valid watched source needs no new filesystem read. Recheck permission before giving the bytes to a caller.

## FluxR tool catalog

This catalog maps common harness capabilities to RelAI tools and protocol actions. It uses the published Codex, Claude Code, OpenCode, and OpenAI API tool sets. Tool names in this catalog are RelAI registry IDs. The registry gives each active tool a dictionary entry with the `⚙` marker. [Codex tool registration](https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/spec_plan.rs) · [Claude Code tools](https://code.claude.com/docs/en/tools-reference) · [OpenCode tools](https://opencode.ai/docs/tools/) · [OpenAI API tools](https://developers.openai.com/api/docs/guides/tools)

For each tool, specify its input schema, output schema, scope, effects, limits, and cancellation behavior. A checked FluxR entry point implements the tool or calls a typed Rust host service. Bind the registry ID to the exact FluxR export and host service signatures. Use the Rust standard library when it provides the operation. For Git, specialized formats, and pattern matching, use embedded Rust libraries as necessary. Tools have no executable fallback.

### File, repository read, and project tools

Implement all the tools in this catalog. Every tool execution uses the shared VM. File reads accept an exact snapshot, a watched source, or a host path as applicable. Use a fresh read for a host path without a valid watch generation. Git tools use admitted repository IDs. Staging tools use a harness-owned temporary area.

| Common capability | FluxR tools to build | Contract |
|---|---|---|
| Working directory, inventory, and metadata | `project.info`, `fs.list`, `fs.stat` | Give the project scope, directory entries, file kind, size, or digest. Use explicit scope for each call. |
| Find and glob | `fs.find` | List paths in the specified source that match a pattern. Give a result limit and a continuation cursor. |
| Read, view text, head, and tail | `fs.read` | Read a bounded line or byte range. Return the actual range and encoding. |
| Grep, file search, and repository search | `fs.search` | Search the specified source by literal text or a bounded regular expression. Return paths, lines, and exact spans. |
| Count, hash, and compare | `fs.count`, `fs.digest`, `snapshot.diff` | Count by a specified rule. Calculate a digest. Compare two admitted snapshots. |
| Write, Edit, Apply Patch, and preview changes | `artifact.write`, `patch.preview`, `patch.stage` | Write a new immutable output. Preview changes against an exact base. Apply changes to a derived snapshot. |
| Make, copy, move, and remove temporary files | `stage.mkdir`, `stage.copy`, `stage.move`, `stage.remove` | Use typed, exact paths in an authorized staging area. Use `std::fs` for the operation. |
| Read and list artifacts | `artifact.read`, `artifact.list` | Read a bounded range of an attachment or output. List artifacts available to the assignment. |
| Git status and refs | `git.status`, `git.refs` | Read the admitted repository state and refs. Disable reuse until the tool tracks all mutable Git dependencies. |
| Git history, tree, show, and diff | `git.log`, `git.tree`, `git.show`, `git.diff` | Read bounded history or objects. Compare exact Git revisions. |
| Select a Git revision for analysis | `git.snapshot` | Make an immutable snapshot from an admitted commit and record its object ID. |
| Project memory and terminology | `project.lookup`, `dictionary.find`, `dictionary.get`, `dictionary.expand`, `dictionary.propose` | Read project evidence or one exact term. Deliver batch definitions in the next turn. Record proposals under the dictionary rules. |
| Tool search, descriptions, and schemas | `tool.find`, `tool.inspect` | Search scoped registry entries. Give the exact contract and activation status. |
| Default tool selection | `tool.promote` | Request an exact qualified default version. Check the expected default, required evidence, and operation authority before updating the registry. |
| Workflow records | `workflow.find`, `workflow.propose` | Search scoped records of recurring work. Propose a candidate with evidence references and checks. |
| Installed skill discovery and resources | `skill.find`, `skill.read` | List permitted skill metadata. Read exact instructions and resources for an assignment with explicit skill use. |
| Plans, todos, and task inspection | `plan.get`, `plan.update`, `task.list`, `task.get` | Read or update the scoped plan. Read assignment state and prerequisites. |
| Session and job inspection | `session.list`, `session.get`, `job.status`, `job.receipt` | Give permitted destinations, states, and recorded results. |
| Run or compose checked tool code | `RUN_TOOL`, `PROPOSE_TOOL`, `TEST_TOOL` | Use the existing protocol. Install and qualify a FluxR version before reusable execution. |

### Capabilities supplied by the relay protocol

Use these protocol actions and states when a common harness tool changes scheduling. Tool code cannot write them through its output interface.

| Common capability | RelAI mechanism |
|---|---|
| Agent, subagent, or TaskCreate | `TASK` creates an assignment and worker. |
| SendMessage, follow-up, or resume with new input | `MESSAGE` gives a delivery to the addressed session. A user instruction can change its scope revision. |
| Ask the user or request a decision | `ASK` makes a question. The required answer is a prerequisite for affected work. |
| Report findings, attach a file, or hand back work | `RESULT` identifies findings or an immutable artifact. A final discriminator result can close its request. |
| Wait for work, receive a callback, or stop a job | `wait_for` and `complete_when` name the conditions. **Stop** records cancellation through the harness. |
| Task updates and completion | Steering changes instructions. The scheduler changes assignment state when prerequisites or acceptance conditions change. |
| Accept work or approve a plan | The user or designated verifier supplies the acceptance receipt. Acceptance gives no new permission. |
| Skill use and project references | An explicit user selection binds a skill to an assignment. A callable workflow uses a qualified FluxR tool. |

### Repository writes, verification, and integrations

| Common capability | FluxR tools to build | Contract |
|---|---|---|
| Apply changes to a checkout | `patch.apply` | Apply a staged patch under the write and recovery rules. |
| Make a directory or write new file bytes | `fs.mkdir`, `fs.write` | Create one specified directory or write one staged artifact to one file. Check the expected destination state. |
| Copy, move, or remove live files or directories | `fs.copy`, `fs.move`, `fs.remove` | Copy or move one regular file. Remove one file or one empty directory. Use the write and recovery rules. |
| Git blame, staging, commit, and worktrees | `git.blame`, `git.stage`, `git.commit`, `git.worktree_create`, `git.worktree_remove` | Use verified in-process Rust Git operations. Each write checks its exact project and expected state. |
| Branch management and Git transport | `git.branch`, `git.fetch`, `git.push` | Create, list, or delete approved branch refs. Fetch and push under the Git transport and write rules. |
| Code navigation and language intelligence | `code.symbols`, `code.definition`, `code.references`, `code.diagnostics` | Use the embedded FluxR analyzer and an embedded Rust analyzer. Use source and available workspace metadata to identify exact spans. Report unresolved references and unavailable analysis explicitly. |
| Semantic retrieval | `project.semantic_find` | Match query terms to project concepts through the dictionary. Rank indexed passages by those concepts. Return exact source references. |
| Format, lint, build, test, and benchmark | `verify.format`, `verify.lint`, `verify.build`, `verify.test`, `verify.benchmark` | Support the checked FluxR verification operations below through the embedded runtime. |
| Data parsing, calculation, tables, and databases | `data.parse`, `data.summarize`, `data.transform`, `data.read_query` | Parse JSON and CSV. Support count, sum, minimum, maximum, and mean. Apply typed row filters and projections. Read SQLite through parameterized query templates. |
| Documents, spreadsheets, notebooks, and media | `document.read`, `document.write`, `sheet.read`, `sheet.write`, `notebook.edit`, `media.read` | Read text from Markdown, DOCX, and PDF. Write Markdown and DOCX. Read and write XLSX cell values and formulas. Edit IPYNB cells. Read PNG and JPEG as scoped media. |
| Archive tools | `archive.list`, `archive.create`, `archive.extract` | List, create, and extract ZIP files. Accept only scoped entries. Extract only to an authorized staging area. |
| Web fetch, downloads, and search | `web.fetch`, `web.search` | Fetch approved HTTPS URLs. Query an approved search service. Record content and source references as artifacts. |
| Repository hosting, issues, and pull requests | `repo.issue_read`, `repo.pr_read`, `repo.pr_propose` | Read GitHub issues and pull requests through an approved connector. Prepare a pull request proposal as a local artifact. |
| Service resources and connectors | `resource.find`, `resource.read`, `connector.call` | List or read resources from registered connectors. Call only named schema-bound operations with current permission. |
| Timers, monitoring, and notifications | `schedule.create`, `schedule.list`, `schedule.cancel`, `monitor.start`, `monitor.status`, `monitor.stop`, `notify.local` | Use the schedule and monitor rules. Send local notifications through the daemon's notification API. |
| Diagnostics and performance | `daemon.metrics`, `job.trace` | Return selected, scoped measurements and receipts. Use in-process measurement APIs. |

For FluxR verification, take an admitted source or exact draft. Require `TEST_TOOL` authority for a draft. Format it with the in-process canonical source formatter. Lint it for contract and host service violations. Build a checked module without activating it. Execute named tests and benchmark cases in the shared VM. Record the actual output, outcome, or measurements. Queue dependent jobs through the scheduler. Record the verification outcome after those jobs finish. Rust source analysis uses the embedded analyzer. A workspace feature that needs a build script or procedural macro requires an admitted generated source or unavailable analysis.

Report `unsupported` for operations outside the stated tool contracts. The implementation must supply every operation inside those contracts. A stub or an unsupported response cannot satisfy catalog acceptance.

### Capabilities without a general tool equivalent

- Shell commands, environment variables, pipes, and general code execution have no direct tool. Compose typed tools and checked FluxR functions.
- Interactive terminals, stdin, process signals, and background processes have no equivalent. Use daemon job state, receipts, and cancellation.
- A request to run Cargo, npm, a compiler binary, or an existing test command needs an embedded implementation. Without one, report `unsupported`.
- Local MCP over stdio and language servers that start processes are outside this design. The same applies to package installers, SSH, containers, and arbitrary remote execution.
- Browser and desktop control are outside this design. Text retrieval through a web tool gives no browser actions.
- Remote code interpreters and media generation APIs do work outside the daemon. They are not substitutes for local FluxR execution.

Map `ls` to `fs.list`, `grep` to `fs.search`, and `cat` to `fs.read`. Map `rm` to the corresponding typed removal operation. `cd` becomes an explicit path in the next request. The harness never runs these command strings.

## RelAI user interface

### Project tabs and relay

Use project tabs in a horizontal bar above the workspace. Give the user **New project** and **Open project** actions in the bar. Use a folder picker for **Open project**. Give the user a setting to put the tabs in a vertical bar on the left. Keep the selected project and tab order after a position change. Provide keyboard navigation and an overflow list for tabs that do not fit.

Put **Relay** and the **Project graph** icon together in the view bar. Open the relay by default. Give the timeline most of the workspace. Keep one composer at the bottom. Show its destination project. Keep a separate draft, timeline position, and graph view for each project. A tab change never transfers a draft to another project.

```text
[Project A] [Project B] [New project] [Open project]  [Settings]
[Relay] [Project graph icon]
┌─────────────────────────────────────────────────────────────┐
│ Relay timeline or project graph                            │
│                                           [Item inspector] │
└─────────────────────────────────────────────────────────────┘
[plan.pdf ×] [reference.png ×]
[Attach files] [Message Project A]                       [Send]
```

In the vertical layout, move the project tabs to the left. Keep the view bar and composer in the workspace.

Show exact submissions, questions, summaries, status, and results in the timeline. A question card shows the work it holds and gives the user **Reply**. A result from an active tool gives the user **Run again**. **Stop** records cancellation. The harness applies the existing cancellation rules and keeps the outcome visible.

Show the branches and their outcomes under each request. Show when the discriminator reviews a fan-out. Let the user inspect the achieved criteria and remaining work. Mark a closed request's result as **Final result**. Give an open request a **Steer** action that selects it in the composer. Send the steering through the normal submission route.

Put a collapsed **Workflow improvement** item under the final result. Show the improvement session's own status. Show the published tool versions, verified gains, or remaining check when the user opens it. Report foreground and improvement costs separately.

Let the user expand an item to see its source, destination, exact payload, term references, prerequisites, attempts, and evidence. Render model and tool output as data. Bind the API to loopback. Use a daemon-issued session credential for every read, write, and event connection. Check the client origin and project scope. Keep this credential out of model inputs and project files. Use the event cursor for reconnection.

In turn details, show the working root and the project instruction files in input order. Show the source version for each file. If instruction loading holds a turn, show the path and cause.

### Composer attachments

Give the user an **Attach files** control and a file drop target in the composer. Show each file's name, size, and capture state. Let the user remove a file before submission. Keep attachments with the draft project when the user switches tabs. Require text or at least one file for **Send**. Hold **Send** while a file is loading or has an error.

Show submitted files with their message or answer in the timeline. Let the user inspect the captured version through the authenticated local API. Show a distinct state for an attachment error that holds a discriminator turn. Use the reply control to attach files to an answer without losing its question binding.

Keep microphone controls, capture, and permission requests outside the UI and the local API.

### Model settings

Open **Models** from the application settings. Open the project scope of the same screen from the project tab menu. Show one row per role. Show the effective model, effort, and their source. Let the user edit the global default or save a project override. Give each override a **Use global default** action. Show the project name during a project edit.

In session details, show the role and preset of the sealed turn. Show the preset for the next turn under current settings. Show when a settings change takes effect. For an unusable preset, show the hold or provider error with a link to the relevant settings.

### Dictionary manager

Put **Manage dictionaries** in the project tab's right-click menu. Give the tab a keyboard action that opens the same menu. Open a dialog for that project. Keep the selected relay open when the dialog closes.

Show owned dictionaries and dictionaries available from other projects. Show the owner and the enabled state for the selected project. Give the user controls to create, rename, link, unlink, enable, disable, and archive dictionaries. Use the dictionary access rules for those changes.

Put a search field and term list next to a definition editor. Show term type, source, aliases, links, and revision. Give the user actions to add, edit, and archive entries. Show the project that will receive the change. Show other enabled links before a change to a shared dictionary. Use authorized erasure when the user asks to erase content.

### Term display and navigation

Render every annotated term as an inline link in the composer, timeline, and definition viewer. Use blue for knowledge, amber for tools, and violet for skills. Use a distinct link style and a visible focus state. Give the link an accessible name with the term, type, and source dictionary. Use colors with sufficient contrast in both themes.

Keep the generated marker characters in model input only. Keep the exact original characters in plain-text copy. Show the definition on activation. Open its links in the same dictionary view. Let **Back** return to the prior definition and focus. A link to a tool or skill opens its definition. It does not call the tool or load the skill.

### Project graph

Open the project graph when the user selects its icon next to **Relay**. Use the main workspace for the graph. Keep the project tabs and bottom composer. Restore the relay's position when the user switches back. Use Gource's animated directory tree as the visual reference. [Gource](https://gource.io/)

Start with the project root. Give every directory its place in the containment tree. Attach each file node to exactly one parent directory. Files at the root attach to the root node. Give separate admitted worktrees their own branches. Keep containment separate from architecture edges.

Provide pan, zoom in, zoom out, fit, path search, and focus on a directory. At low zoom, group files under their directories and show their counts. Show file names and edges as the user zooms in. Keep node positions stable when small changes arrive. Use a reduced-motion mode.

Click a directory node to open its contents and breadcrumb in the inspector. Click a file node to open its data in the internal file viewer. Show the path, source version, and relevant actions or changes. Open a saved node at its recorded snapshot. For a live node, use its recorded generation. If the generation changed, show that state and offer **Open current**. The viewer uses scoped reads and the file cache. It does not start another application.

#### Graph layers and evidence

| Layer | Display |
|---|---|
| Files and directories | Show the containment tree from an admitted snapshot or reconciled live inventory. Show staged changes with a separate state. |
| Architecture | Show components, declared boundaries, and dependency edges. Each edge identifies its source and version. Distinguish parsed evidence, user declarations, and session proposals. |
| Activity | Show observed file changes and recorded session or job events. Link each event to its relay record, effect receipt, or watcher observation. |

Let the user filter layers by path, component, assignment, or source. A selected component shows its files, dependencies, source evidence, and associated work. A selected edge opens its evidence. Report unresolved references. A file change does not establish a dependency change. Mark the affected analysis stale until a check of the new source completes.

Show a session at a file only when a record links that session to the file. Distinguish assigned work, a recorded read, a staged change, and a committed write. Animate node additions, recorded renames, and recent activity. An external watcher event can show the changed path with an unknown author. Do not infer an author from timing.

#### Live view and history

Use the project event cursor for graph updates. Bind each graph subscription to a project, source version, and filter set. A graph response records its subscription identity and cursor. Give each delta the prior cursor and the new cursor. Send a cursor advance even when an event leaves the projection unchanged. Apply updates only to the current subscription and prior cursor. Load a new graph snapshot if that cursor is unavailable. On a lost watch, show **Reconciling**. Keep the last observed structure with its source label until reconciliation completes.

Provide **Live**, **Pause**, and a time control for recorded graph states. Pausing the view does not pause the project. Let the user inspect an admitted Git snapshot or replay recorded activity. Label the source and time range. Git history and live observations retain separate provenance. Show gaps in history when there is no recorded state. Provide **Compare** for two admitted snapshots. Show added, removed, and changed paths. Let the user add a selected source reference to the composer. Keep it distinct from a submitted attachment.

Use project-scoped node IDs. Equal-content files remain separate nodes. Keep a node ID across a rename only when a record identifies both paths. If the evidence is insufficient, record a removal and an addition. Keep the user's zoom and selection when those objects remain available.

Load visible branches and selected architecture edges in bounded groups. Provide a keyboard-accessible tree and relationship list for the same data. Keep the graph, inspector, and list selection synchronized. Return focus to the graph control when the user closes the view.

## Parallel implementation

Set the shared interfaces and test data first. Supply a test provider, sample continuation files, dictionary cases, and test doubles for storage and the tool runtime. The integrator owns shared types and acceptance criteria. Give each package one writer. Follow the owner's concurrency and repository rules.

| Package | Responsibility | Interface and checks |
|---|---|---|
| Protocol and fixtures | Owns wire schemas, IDs, span types, fan-out review records, and shared interfaces. | Check the role catalog, attachment manifests, action variants, and scripted scenarios. |
| Storage and snapshots | Owns SQLite migrations, atomic commits, event cursors, files, preset records, and immutable artifacts. | The store accepts expected revisions and mutation records. Check atomic submissions, turn seals, replay, and file recovery. |
| Scheduler and relay rules | Owns action admission, questions, inboxes, prerequisites, turn seals, fan-out reviews, and request closure. | Use test doubles for review readiness, revision races, state changes, and retries. |
| Workflow improvement | Owns completion evidence selection, candidate records, promotion checks, and reuse measurements. | Use the closure event, scheduler, registry, and project context contracts. Test one session per completed request and publication races. |
| Shared execution and source watches | Owns single-flight, cache keys, the file cache, watch generations, and reconciliation. | The scheduler calls the shared index before dispatch. The tool services use the file cache. Test with controlled watch events and I/O counts. |
| OpenAI adapter | Owns Responses API requests, attachment encoding, model capability checks, and usage records. | The provider takes a sealed `TurnRequest` and returns an `AttemptOutcome`. It does not dispatch actions. |
| Dictionary and context | Owns dictionaries, markers, project instruction discovery, and context assembly. | Return scoped definitions and ordered turn inputs. Use the shared source services for discovery and reads. |
| Skill installation and use | Owns package checks, installation records, version bindings, and scoped resource access. | Use the Git, archive, dictionary, and context contracts. Test installation and explicit use separately. |
| Tool runtime and registry rules | Owns FluxR admission and execution, host interfaces, qualification checks, and tool receipts. | The runtime takes a pinned version or draft, inputs, limits, and tests. The scheduler commits activation through the store. |
| Built-in tool catalog | Owns checked FluxR entry points and Rust services by capability group. | Use the shared host interfaces. Qualify each tool with its actual service and no child processes. |
| Filesystem and repository effects, and connectors | Owns host and repository write recovery, Git transport, external operations, credentials, and effect receipts. | Use the shared write and connector contracts. Test conflicts, interrupted writes, and uncertain external results. |
| Project graph | Owns node identity, tree projection, evidence links, and recorded graph states. | Use the watcher, snapshot, project map, and relay event contracts. Test graph data and transitions. |
| Relay UI | Owns tabs, composer attachments, model settings, dictionary manager, timeline, and graph views. | Use the authenticated local API and event cursor. Use non-image tests for layout and accessibility behavior. Review visuals manually. |

Each package can use the fixtures and test doubles after the integrator sets the shared interfaces. Integrate storage and scheduling first. Then connect the dictionary, tool runtime, adapter, and UI. Send shared schema changes through the integrator. Do a check of the scripted scenario before you use the live provider.

## Acceptance tests

Map every registry ID to a successful case and an applicable error case. Use the actual service for the supported operation. Use test doubles for routing and failure injection. A test double cannot prove native or connector behavior.

Complete the implementation only when every catalog entry and the system tests pass. Test the supported formats and operations from their contracts. Keep the tool map and the acceptance results together.

### Post-result workflow improvement

1. Complete two requests close together. Interrupt final publication and restart. Check that each final result yields exactly one improvement session.
2. Give the first session a short budget. Hold a draft test with a test double. Check that other requests can proceed.
3. Try a `TASK` and a final `RESULT` from the improvement session. Check that the action schema rejects both. Check that an improvement report starts no session.
4. Have two completed requests expose the same tool defect. Check that the sessions share the candidate and test evidence.
5. Reproduce the defect on the existing default. Correct it with a new FluxR version. Require an independent check and the existing contract checks.
6. Attempt promotion with an old expected default and with a wider contract. Check that neither request changes the default.
7. Promote the compatible repair. Check the registry and dictionary binding. Check that a sealed invocation keeps its original version.
8. Look up the repair in a later request. Run it directly. Check the output and verify that typed execution requires zero model calls.
9. Finish a routed request with a direct tool receipt. Check that its result appears before the improvement session starts. Check that **Run again** starts no improvement session.
10. Give a session no reproducible improvement. Check that it reports no change. Check that it creates no tool or workflow binding.
11. Compare the same verified work before and after adoption. Check that measurements distinguish estimates and include the improvement session's cost.
12. Close a routed request from a tool receipt. Count one initial discriminator turn and zero tool model calls. Count the improvement session separately.
13. Fail a direct routed tool call after its retries end. Check the failed state and the absence of a final success or improvement session.

### Fan-out review and final result

1. Route a request into two assignments. Finish one while the other runs. Make sure the review waits.
2. Finish the second assignment. Check that one discriminator review receives both outcomes and the remaining criteria.
3. Have a worker create a nested fan-out. Settle its successful review. Check that it releases the ancestor review.
4. Fail a branch and leave another waiting for a reply. Check that the review can dispatch independent repair work and preserve the wait.
5. Have a follow-up `TASK` replace the failure. Check the request and parent IDs. Make sure it preserves the required criteria.
6. Give the discriminator an answer to its review question. Make sure it reaches only the addressed request. Do not repeat a review while its wait conditions stay the same.
7. Try a worker's final result, a final result with a task, and a final result with an unmet criterion. Make sure none closes the request.
8. Satisfy the last criterion after all workers finish. Check that one review can propose the final result.
9. Submit targeted steering before the final result commits. Make sure the old proposal cannot close the revised request. Submit unrelated work and verify no closure delay.
10. Interrupt review scheduling and final file publication. Restart. Check one logical review per state and one final file with its relay event.
11. Finish the request. Check that the relay can accept unrelated work and that the final result does not close other requests.
12. Delay publication of the final file. Publish unrelated relay events. Resume publication and check that a connected client gets the final event once.
13. Revoke access before publishing a final file. Check that one unavailable event reaches the relay. Check that the improvement session excludes the revoked content.

### Project instruction loading

1. Open an existing checkout with a modified `AGENTS.md` and an untracked `Claude.md`. Check that opening leaves its worktree and index intact.
2. Inspect the first discriminator request and a worker request. Make sure both start with the two complete project instructions before regular input.
3. Check the exact `PROMPT.txt` bytes. Check the instruction order, scope, source versions, and digests in the sealed turn record.
4. Give one assignment a directory with another instruction file. Make sure the file applies to that directory and stays out of unrelated input.
5. Discover a new target directory during a turn. Make sure an affected write waits until a new turn receives those instructions.
6. Edit one root file and remove the other during a call. Check a retry and the next turn. Make sure only the next turn gets the new selection.
7. Attach a file named `AGENTS.md` to a message. Make sure it stays in the attachment input section and has no project instruction authority.
8. Add conflicting rules in the two root files. Make sure the relay asks about affected work and dispatches clear independent work.
9. Try an unreadable file and a file that exceeds the input limit. Make sure the turn holds and shows the cause.
10. Discover a new instruction scope through a resolved write target. Replace the held action. Check that only the replacement can execute.

### Attachments and discriminator routing

1. Add `plan.pdf` and `reference.png` to one project draft. Switch projects. Make sure each draft keeps its own files.
2. Submit exact text with both files. Check `PROMPT.txt`, `attachments.json`, and the stored artifact digests against the submitted data.
3. Change or remove the original files. Check the first `TurnRequest`. Make sure it includes the prompt and both captured files in order.
4. Replay and retry that turn. Make sure replay sends no request. Make sure a retry uses the same attachment digests and input representations.
5. Route two independent assignments. Give one the ID of `plan.pdf`. Make sure the other receives neither attachment.
6. Attach a file in **Reply** and add a separate instruction. Make sure only the addressed reply carries that file. Dispatch the independent work.
7. Submit a file with no text. Make sure the prompt file is empty. Ask for the intended work if it is unclear.
8. Try an unsupported or oversized file. Make sure the draft keeps the file and shows the cause. Verify no partial request is sent.
9. Change to an incompatible model after submission but before the turn seal. Make sure the turn holds and the submission stays intact.
10. Check that the composer and API have no microphone capture or permission flow.

### Per-role model settings

1. Start every role with its default preset through the test provider. Check the role, model, and effort in the turn records and provider requests.
2. Change the project override during an active turn. Make sure the active turn and its retry use the sealed preset.
3. Seal an already queued turn and resume a waiting session in the same role. Make sure both use the override. Make sure the save alone starts no turn.
4. Check a different role and a different project. Make sure the override changes neither preset.
5. Save a new global default. Remove the project override. Make sure the next turn uses the new global default.
6. Test a concurrent save and turn seal in both commit orders. Make sure the recorded revision establishes the effective preset.
7. Reject an invalid effort. Simulate provider loss of model access. Make sure the affected turn reports the cause and uses no fallback.
8. Change the discriminator override. Make sure the next submission uses it. Make sure no deterministic tool job requires a model preset.
9. Replace an unaccepted turn on user request. Check the new job ID and preset. Make sure the old turn cannot deliver actions.

### FluxR tool and reply

1. Submit a Markdown link-check assignment with a FluxR tool that resolves `../contact.md` incorrectly.
2. Use a fixture with `docs/guide/start.md` and `docs/contact.md`. The correct output has no finding for that link.
3. Make sure the test receipt shows the exact difference. Make sure the draft remains inactive.
4. Make the worker ask whether to include `examples`. Restart RelAI while the worker waits.
5. Answer “Yes, include examples. Also count the Markdown files in docs.”
6. Make sure the harness delivers one addressed reply with a summary. Make sure the independent count job can finish.
7. Have the worker supply the corrected FluxR tool, a qualification request, and a dependent invocation.
8. Make sure the tests give the required acceptance evidence. Make sure the harness activates the exact version. Show the invocation result without a model call. Count any later discriminator review separately.
9. Repeat the qualification with a tool that always gives the example answer. Make sure a different trusted input causes it to fail.
10. Retry a failed turn that included an answer. Make sure the retry keeps the answer. Consume it only on an accepted outcome.

### Single VM and native services

1. Run a built-in tool, a draft test, and a qualified FluxR tool. Make sure they use the same daemon and portable VM route.
2. Submit shell text, a native Rust declaration, and an unauthorized host service call. Make sure admission rejects each one.
3. Run a tool until it reaches its fuel limit. Repeat with a cell limit and a bad handle. Make sure the next valid invocation succeeds.
4. Make one invocation create call state and handles. Make sure a later invocation cannot read them after success or failure.
5. Use a Git fixture with `git.exe` absent from `PATH`. Read its refs, trees, and blobs. List and read files with the Rust services. Remove an authorized temporary test file. Make sure the daemon starts no child process in any case.
6. Read a file outside the project roots twice by host path. Make sure each read uses the Windows account and fresh data without a watch.
7. Try the same access with an escaping project-relative path, an unexpected link, and a path the Windows account cannot read. Check the distinct failures.
8. Make a host file change with an exact expected state. Repeat with the old state. Make sure the second request cannot overwrite the change.
9. Try to read private daemon storage with a generic file tool. Make sure the typed record APIs still enforce the record scope.

### Cache and watch recovery

1. Send the same cold directory request from several sessions. Make sure one VM job starts and each caller receives a delivery and wake. Count request-time directory reads separately from watcher scans.
2. Repeat the request. Make sure the hit starts no VM job and does no filesystem read.
3. Cancel one caller during a shared job. Make sure the other callers still get the result.
4. Read overlapping byte and line ranges. Count disk reads. Make sure cached blocks are not read again.
5. Add, remove, rename, and replace files. Include a same-size write. Make sure the affected entries no longer serve hits.
6. Change a file during a read. Make sure the old result cannot fill the cache after invalidation.
7. Simulate overflow and root replacement. Make sure live cache hits stop until the affected scope completes reconciliation. Make sure a fresh request performs the requested I/O.
8. Evict data by exceeding the LRU budget. Make sure an evicted file reloads correctly and an unchanged cached file needs no read.
9. Try unequal permission scopes and a write tool. Make sure the daemon does not share results in either case.
10. Change a live file that also exists in a snapshot. Make sure the old snapshot still returns its original bytes.
11. Return a partial tool result or read a source outside its shared key. Make sure neither result can serve a shared cache hit.

### Writes, transport, and scheduling

1. Change an expected file or ref before a patch or Git write. Make sure the harness reports the conflict before modification.
2. Stop a multi-file effect partway through. Make sure recovery accounts for each completed write and keeps an unrelated external edit.
3. Stage selected files with unrelated changes in the index. Make sure the commit preserves the unrelated staged changes.
4. Fetch and push against an approved HTTPS Git fixture. Make sure push rejects a changed destination or non-fast-forward update.
5. Try an executable hook, helper, or filter in Git configuration. Make sure the daemon starts no process.
6. Interrupt an external write after the service can accept it. Make sure recovery uses an exact state check or idempotency key.
7. Restart when a schedule occurrence is due. Make sure its ID appears once. Revoke authority and confirm that subsequent dispatch stops.
8. Erase authorized content with a pending cache entry and exported file. Make sure no lookup, cache hit, or replay supplies the erased bytes.
9. Delay a remote write after its local deadline. Give the daemon a read that still shows the old state. Make sure it holds the retry.

### Dictionary and markers

1. Add a business definition during the link-check work. Add a tool entry for the registered version.
2. Make sure autocomplete shows the description above the suggestions. Make sure the three types have distinct styles and accessible names.
3. Use emoji, repeated terms, adjacent terms, punctuation, and literal symbols in the sample. Use composed and decomposed Unicode accents.
4. Make sure the model input reproduces the original text. Make sure each marked span gives the intended exact revision.
5. Try ambiguous names, overlapping spans, invalid byte ranges, and an edit inside a selected span.
6. Make sure ambiguous names have no automatic marker. Make sure invalid spans cannot become markers.
7. Edit a definition as the user. Make a session propose a conflicting change.
8. Make sure the human wording remains active. Make sure an earlier turn still resolves to its earlier revision.
9. Edit a tool description. Make sure the executable digest and callability remain the same. Mention a draft and a literal marker. Make sure neither starts a tool.

### Skill installation and use

1. Install a local package and a package from an approved HTTPS Git repository. Make sure the receipts record exact package identities.
2. Select the installed name in autocomplete. Make sure its definition opens and selection alone loads no instructions. Make sure only model input has the generated `✦` character.
3. Use the skill explicitly for an assignment. Make sure only affected turns receive the exact version and permitted references.
4. Install a package with a FluxR draft. Make sure installation starts no tool. Make sure the draft must qualify before invocation.
5. Try a package with a path escape or a requirement to run a shell. Make sure the package cannot grant that access.
6. Update the skill after a human edits its definition. Make sure the protected wording remains. Make sure the pinned assignment keeps its version.
7. Remove the skill. Make sure the harness blocks a new use and reports any active work that still needs it.

### Tabs, linked dictionaries, and term expansion

1. Create two projects. Make sure each has an enabled owned dictionary. Put a different draft in each composer.
2. Move the tab bar from the top to the left. Make sure the active project and both drafts remain correct.
3. Open **Manage dictionaries** through the tab menu. Link the other project's dictionary. Make sure its terms appear in suggestions.
4. Give both dictionaries the same term name. Make sure the user can select the intended source. Make sure automatic matching leaves the name unbound.
5. Open a linked tool or skill definition. Make sure the definition grants no invocation or skill use in the receiving project.
6. Edit the linked definition through its owner scope. Make sure the dialog shows the destination and other enabled projects.
7. Link definition A to B and B to A. Follow the links and use **Back**. Make sure navigation opens one definition at a time.
8. Request A, B, and A in an expansion. Remove access to B before delivery. Make sure the next turn gets A once and B's unavailable outcome.
   Repeat after sealing the turn. Check that the daemon retires the sealed turn before a new provider attempt.
9. Submit an expansion that exceeds the turn limit. Make sure the tool reports the limit and gives no partial definitions.
10. Activate the three term types with the keyboard. Make sure the UI opens definitions and shows no generated marker characters.

### Project graph interaction

1. Open the graph from the view bar. Zoom and pan. Make sure files remain attached to their parent directories.
2. Select a directory and file. Make sure the inspector opens the correct listing or file at the represented source version.
3. Change a live file before opening its node. Make sure the old node cannot silently open the new contents.
4. Copy a file and then rename the original. Make sure equal bytes do not merge the nodes. Keep the renamed identity only with evidence.
5. Add a staged patch, a committed write, and an external change. Make sure their activity states differ. Give the external change no assumed author.
6. Change an import. Make sure the old architecture edge shows its stale state. Make sure new evidence identifies the new edge.
7. Lose a watcher and reconnect after an event gap. Make sure the graph shows its observed state and reconciles before it shows current data.
8. Pause the graph and continue project work. Resume the graph. Make sure current activity catches up without stopping jobs.
9. Compare two admitted snapshots. Open an old file from the comparison. Make sure it opens the old bytes.
10. Use the accessible tree to select a node. Make sure the graph and inspector show the same selection. Use non-image assertions.
11. Change filters while a graph response is pending. Reject the old response. Process an unrelated event with a cursor advance.

### Reuse and recovery

1. Use a snapshot with links to absent targets `docs/absent.md` and `examples/missing.md`. The qualified tool gives two findings.
2. Restart RelAI. Add `examples/missing.md` to a new snapshot. Select **Run again** with that snapshot.
3. Make sure one finding remains. Make sure the executable identity is the same and the model-call count is zero.
4. Send invalid typed arguments. Then revoke the tool and send valid arguments. Both requests must fail before execution without a model call.
5. Inject failures near the delivery commit, file publication, and attempt start. Make sure recovery gives exactly one inbox entry for the committed answer.
6. Simulate an uncertain provider outcome. Make sure replay does not send a new call.
7. Commit steering during a worker's older turn. Make sure the older result cannot release work under the new revision.
8. Add unrelated relay history. Make sure equivalent turn inputs remain the same size and contain the same items.
9. Resend a submission under its original key. Check that it gives one submission and one intake delivery. Reject changed bytes under that key.
10. Restart after a tool becomes revoked. Check that its stale result cannot enter another turn. Keep already committed effect receipts.

### Live provider

Use the live provider on a limited version of the scripted scenario. Include a file attachment and a configured reasoning effort. Record missed instructions, wrong destinations, duplicate logical deliveries, verified results, wait times, and model-call counts. Compare separable and tightly coupled assignments with a single-worker baseline before increasing automatic delegation.
