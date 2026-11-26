# RelAI

RelAI opens an existing Windows project as a permanent local relay. Its application modules are checked FluxR 1. A reusable native host supplies the portable interpreter, SQLite, loopback HTTP, file I/O, and provider transports. Project files stay in their existing checkout. The default data directory is `%LOCALAPPDATA%\RelAI`.

## Run on Windows

Install Git, Rust with the MSVC build tools, PowerShell 7, and Codex CLI. Sign in to Codex with ChatGPT. From the repository run

```powershell
pwsh -File .\Start-RelAI.ps1
```

Open the local URL printed by the launcher. Its fragment contains the session credential. Choose **Open project**, browse to the folder, and send a message. The launcher starts a capability-token-protected Codex App Server on loopback. RelAI checks that Codex reports a ChatGPT account before each subscription turn. Codex owns account credentials. Each attempt starts a fresh ephemeral thread with its sealed model, reasoning effort, input, and output schema. A pending direct model result appears with an **Accept result** control.

A project can select a different provider for each role under **Models**. Set the provider environment variables before starting RelAI. The application records a job error when the selected deployment rejects its model or input.

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

The first command checks formatting, runs strict Clippy, and executes the offline integration suite. The Git transport fixture uses Python 3 and Git to run a disposable HTTPS reference server on loopback. The second starts the local Codex service and runs a bounded subscription test with a captured text attachment. Anthropic and Qwen integrations have not been exercised against endpoints.

## Application and runtime

The source under `app/` contains the checked FluxR relay, scheduler, provider adapters, dictionary, tools, and view definitions. The application manifest declares public commands, provider destinations, and permitted continuations. The host under `runtime/` supplies SQLite, the portable interpreter, operating-system services, codecs, HTTP/JSON-RPC transport, and a browser renderer. User-authored tools execute in a separate bounded portable invocation with no native imports.

Write tools first produce an immutable proposal. The local user supplies a separate typed grant for its digest. Windows file effects use TxF for supported local volumes. Protected checkout patches currently require a detached HEAD or a loose branch reference. Named HTTP connectors have a fixed destination, method and argument schema. A POST requires an exact operation grant. Schedules and monitors can run selected read tools without a model call.

Git staging consumes selected artifact versions. Commits preserve the existing index and include only selected staged paths. A managed linked worktree is a detached checkout under RelAI's private directory on a compatible NTFS volume. Removal checks ownership, contents, index, metadata and reachability, and retains a commit anchor. Git fetch and push each operate on one registered HTTPS repository and branch. Push requires a fast-forward from the exact approved old tip. The daemon uses an embedded Git implementation. [Git Smart HTTP protocol](https://git-scm.com/docs/gitprotocol-http)

A skill package can include `relai-skill.json` with pinned `tool_refs` and `tool_drafts`. Installation retains the source, contract and test files. The user can inspect a draft and explicitly propose it through `skill_draft`. Installed packages can also be pinned to a commit and package directory from an approved fetch. [Package admission tests](runtime/tests/project_context.rs)

The pinned FluxR language provides checked UTF-8 literals and compiles entry points across source documents. Each entry retains its reachable functions and host signatures. The same interpreter runs the application and the separately bounded authored tools. [FluxR UTF-8 support](https://github.com/Ryd-rb/FluxR-lang/commit/69e1898934f8954f6f03a51bd7bc9ffc1ed3b620) · [FluxR application linker](https://github.com/Ryd-rb/FluxR-lang/commit/62e412b1b7aa34cad1c8472d7f974e0bfc111552)

Use `-ExternalProvider` to launch without starting a Codex service when using only Anthropic or Qwen. Project state, grant records, and artifact bytes are local. Keep the launch URL and the state directory private.

## Acceptance

[Implementation and test coverage](ACCEPTANCE.md) records the verified behavior and remaining work against the [design](DESIGN.md).
