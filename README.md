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

The first command checks formatting, runs strict Clippy, and executes the offline integration suite. The second starts the local Codex service and runs a bounded subscription test with a captured text attachment. Anthropic and Qwen integrations have not been exercised against endpoints.

## Application and runtime

The source under `app/` contains the checked FluxR relay, scheduler, provider adapters, dictionary, tools, and view definitions. The application manifest declares public commands, provider destinations, and permitted continuations. The host under `runtime/` supplies SQLite, the portable interpreter, operating-system services, codecs, HTTP/JSON-RPC transport, and a browser renderer. User-authored tools execute in a separate bounded portable invocation with no native imports.

Write tools first produce an immutable proposal. The local user supplies a separate typed grant for its digest. Windows file effects use TxF for supported local volumes. Protected checkout patches currently require a detached HEAD or a loose branch reference. Named HTTP connectors have a fixed destination, method and argument schema. A POST requires an exact operation grant. Schedules and monitors can run selected read tools without a model call.

Use `-ExternalProvider` to launch without starting a Codex service when using only Anthropic or Qwen. Project state, grant records, and artifact bytes are local. Keep the launch URL and the state directory private.

## Acceptance

[Implementation and test coverage](ACCEPTANCE.md) records the verified behavior and remaining work against the [design](DESIGN.md).
