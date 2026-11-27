# RelAI

RelAI opens an existing Windows project as a permanent local relay. Its application modules are checked FluxR 1. The desktop window uses dark native Windows controls. Checked FluxR modules define its screens, actions, and state. Project files stay in their existing checkout. The default data directory is `%LOCALAPPDATA%\RelAI`.

## Run on Windows

Install Git, Rust with the MSVC build tools, PowerShell 7, and Codex CLI. Sign in to Codex with ChatGPT. From the repository run

```powershell
pwsh -File .\Start-RelAI.ps1
```

Select **Choose folder…** to open an existing project. Each project has its own chat and draft. When switching projects, **Back** or cancelling the folder picker returns you to the previous view with your draft and its destination intact. Press **Enter** to send and **Shift+Enter** for a new line. **Ctrl+Enter**, **Send**, file drops, and the file picker also work. New answers follow the chat while you are at the bottom. Scroll up to keep reading in place during updates. On a touchscreen, drag vertically over a chat message to scroll. With a mouse, drag across message text to select it for copying. The project heading and compact message box stay in place while you read when there is room. Send becomes available when the draft contains text or a file. **Earlier messages** opens the preceding page of history.

RelAI shows your messages, its questions, and its answers in the chat. The composer names the question it will answer when one is open. Open questions appear together with their suggested answers. The selected question is marked in the transcript. If several questions are open, choose one in the composer or select **Reply** beside it. **New topic** keeps your draft and clears its reply target. Selecting a suggested answer puts it in the composer so you can edit it before sending.

A direct answer offers **Accept answer** and **Request changes**. When the latest request has one answer ready for review, the composer offers to revise it. After acceptance, your next message follows up on that answer even if an older request is waiting for a reply. **Other questions** lets you choose an earlier question from the composer while keeping your draft. The model receives the selected answer, its user message and your change request. Accepting marks that answer in the transcript and lets you ask a follow-up. The composer names the original topic throughout. **Request options** follows the latest message in each active request. It lets you update or stop work. While you wait, chat shows whether RelAI is inspecting files, searching, or waiting for a model response. Open **Activity** for delivery records and diagnostics.

When you ask about the open project, RelAI can list its folders, read a text file or search that file for a phrase. Each read appears in Activity. When a turn requests independent project reads or checks the status of several proposed changes together, RelAI collects their receipts and resumes the conversation once. You can ask about a README or build configuration without attaching it. When you answer a question, the session that asked receives your reply alongside the original request and the exact question. If it asked several questions in one turn, it can wait for the remaining replies and combine the answers.

Ask RelAI to edit a supported Git checkout to receive a patch in the conversation. Select **Review patch** to inspect each file's diff. **Approve and apply patch** authorizes that exact proposal and applies it as a single Windows filesystem transaction. The card reports the outcome. When the Git revision or an affected file has changed, ask for a fresh proposal.

The launcher starts a capability-token-protected Codex App Server on loopback and checks for a ChatGPT sign-in before each subscription turn. Codex owns the credentials. Each attempt opens a fresh ephemeral thread with its sealed input, model, reasoning effort, and output schema.

The sidebar groups Relay, Activity, Project graph and Tools under **Workspace**. Models and Dictionary are under **Settings**. Each project remembers the tab you selected on each page while the app is open. Relaunching opens the project you last selected. Existing installations without a saved selection reopen the project of your latest message.

- **Activity** lists the most recent 100 events. Select a row to inspect it or choose **Needs attention** to filter the list.
- **Project graph** opens on **Files**. Browse a retained snapshot or use **Relationships** to inspect its connections.
- **Tools** opens on a searchable **Library**. Select a result to prepare its arguments. **Versions**, **Changes** and **Automation** group the available operations.
- **Models** shows the active assignments. Select a role and open **Configuration** to edit its model, provider, reasoning effort and preset scope. Set provider environment variables before launch.
- **Dictionary** searches terms from the project's linked dictionaries as you type. Open the selected definition to read or edit it. **Dictionaries** and **Skills** contain their respective management controls.

To start on a particular page, run `pwsh -File .\Start-RelAI.ps1 -View models`. Accepted views are `relay`, `activity`, `graph`, `tools`, `models`, `dictionary` and `open`.

| Provider | Configuration | Wire protocol |
|---|---|---|
| Codex Pro (default) | ChatGPT sign-in through Codex CLI | Local Codex App Server JSON-RPC with structured turn output |
| Anthropic | `ANTHROPIC_API_KEY`, exact Claude model ID | Anthropic Messages with adaptive thinking and structured output |
| Self-hosted Qwen | `RELAI_QWEN_CHAT_URL` set to the complete chat-completions endpoint, exact served model ID, optional `RELAI_QWEN_KEY` | OpenAI-compatible Chat Completions with JSON Schema and reasoning effort |

Use HTTPS for remote deployments. A local Qwen server can use loopback HTTP. Select a deployment that supports the requested effort, structured output, and attachment media. Text and images have provider-specific representations. Anthropic also supports a PDF document part. The Codex adapter uses the experimental WebSocket interface.

## Tests

```powershell
pwsh -File .\Test-RelAI.ps1
pwsh -File .\Start-RelAI.ps1 -CheckProvider
```

The first command checks formatting, runs strict Clippy, and executes the offline integration suite. The Git transport fixture uses Python 3 and Git to run a disposable HTTPS reference server on loopback. The second command starts the local Codex service and runs two subscription checks. One verifies an attached word, final delivery and improvement in two model turns. The other resumes an existing upload question in a disposable project. It checks that RelAI routes the reply, reads the project README and returns its unique value within five model turns. Anthropic and Qwen integrations have not been exercised against endpoints.

## Application and runtime

The source under `app/` contains the checked FluxR relay, scheduler, provider adapters, dictionary, tools, and view definitions. The application manifest declares public commands, provider destinations, and permitted continuations. The host under `runtime/` supplies SQLite, the portable interpreter, operating-system services, codecs, and HTTP/JSON-RPC transport. The checked desktop modules turn dashboard data into keyed control descriptions and handle edits, navigation, and command results. The reusable FluxR Win32 library creates the native controls, handles Windows messages, and reuses controls across updates. The window and controller run on separate threads. [FluxR native window contract](https://github.com/Ryd-rb/FluxR-lang/blob/6fe5f1ce7cf77f672208b7e45e3e970eae3db0a7/docs/native-windows-ui.md) User-authored tools execute in a separate bounded portable invocation with no native imports.

Tools use stable dictionary entries. Promoting a qualified version checks the expected current default and its capability. It creates a new binding revision and preserves human-edited wording. A project's owner can review proposed wording changes in **Dictionary**. Changing a definition leaves executable versions under registry control.

Write tools first produce an immutable proposal. The local user supplies a separate typed grant for its digest. Windows file effects use TxF for supported local volumes. Protected checkout patches currently require a detached HEAD or a loose branch reference. Named HTTP connectors have a fixed destination, method and argument schema. A POST requires an exact operation grant. Schedules and monitors can run selected read tools without a model call.

Git staging consumes selected artifact versions. Commits preserve the existing index and include only selected staged paths. A managed linked worktree is a detached checkout under RelAI's private directory on a compatible NTFS volume. Removal checks ownership, contents, index, metadata and reachability, and retains a commit anchor. Git fetch and push each operate on one registered HTTPS repository and branch. Push requires a fast-forward from the exact approved old tip. The daemon uses an embedded Git implementation. [Git Smart HTTP protocol](https://git-scm.com/docs/gitprotocol-http)

An owner can grant or revoke a live external file for a single request revision and session through **Tools → Changes → File changes → Select or revoke a live source**. Supply `op` (`allow`, `list`, or `revoke`), `request`, `revision`, and `host_path` for a file operation. The optional `session` defaults to the project's discriminator. A queued read rechecks the selection before issuing I/O and before admitting its result. A normal attachment captures the submitted bytes and keeps its original host path private.

A skill package can include `relai-skill.json` with pinned `tool_refs` and `tool_drafts`. Installation retains the source, contract and test files. The user can inspect a draft and explicitly propose it through `skill_draft`. Installed packages can also be pinned to a commit and package directory from an approved fetch. [Package admission tests](runtime/tests/project_context.rs)

The pinned FluxR language supports raw multiline UTF-8 text with `@text` and grouped accumulator calls with `@build`. The desktop views use both for control descriptions and queries. The compiler links entry points across source documents. Each entry retains its reachable functions and host signatures. The same interpreter runs the application and the separately bounded authored tools. [FluxR UTF-8 support](https://github.com/Ryd-rb/FluxR-lang/commit/69e1898934f8954f6f03a51bd7bc9ffc1ed3b620) · [FluxR application linker](https://github.com/Ryd-rb/FluxR-lang/commit/62e412b1b7aa34cad1c8472d7f974e0bfc111552)

Use `-ExternalProvider` to launch without starting a Codex service when using only Anthropic or Qwen. For local API integrations, use `-Headless` and optionally `-Port`. Headless mode prints the loopback origin and a per-launch credential for authenticated API calls. Keep the credential and the state directory private.

## Acceptance

[Implementation and test coverage](ACCEPTANCE.md) records the verified behavior and remaining work against the [design](DESIGN.md).
