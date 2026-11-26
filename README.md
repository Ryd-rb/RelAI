# RelAI

RelAI opens an existing Windows project as a permanent local relay. Its application modules are checked FluxR 1. A reusable native host supplies the portable interpreter, SQLite, loopback HTTP, file I/O, and provider transports. Project files stay in their existing checkout. The default data directory is `%LOCALAPPDATA%\RelAI`.

## Run on Windows

Install Git, Rust with the MSVC build tools, PowerShell 7, and Codex CLI. Sign in to Codex with ChatGPT. From the repository run

```powershell
pwsh -File .\Start-RelAI.ps1
```

Open the local URL printed by the launcher. Its fragment contains the session credential. Choose **Open project**, enter the folder, and send a message. The launcher starts a capability-token-protected Codex App Server on loopback. RelAI checks that Codex reports a ChatGPT account before each subscription turn. Codex owns account credentials. Each attempt starts a fresh ephemeral thread with its sealed model, reasoning effort, input, and output schema. A pending direct model result appears with an **Accept result** control.

A project can select a different provider for each role under **Models**. Set the provider environment variables before starting RelAI. The application records a job error when the selected deployment rejects its model or input.

| Provider | Configuration | Wire protocol |
|---|---|---|
| Codex Pro (default) | ChatGPT sign-in through Codex CLI | Local Codex App Server JSON-RPC with structured turn output |
| Anthropic | `ANTHROPIC_API_KEY`, exact Claude model ID | Anthropic Messages with adaptive thinking and structured output |
| Self-hosted Qwen | `RELAI_QWEN_CHAT_URL` set to the complete chat-completions endpoint, exact served model ID, optional `RELAI_QWEN_KEY` | OpenAI-compatible Chat Completions with JSON Schema and reasoning effort |

Use HTTPS for remote deployments. A local Qwen server can use loopback HTTP. Select a deployment that supports the requested effort, structured output, and attachment media. Text and images have provider-specific representations. Anthropic also supports a PDF document part. The Codex adapter uses the experimental WebSocket interface.

## Tests

```powershell
cargo test --workspace --locked
pwsh -File .\Start-RelAI.ps1 -CheckProvider
```

The first command runs offline checks. The second starts the local Codex service and runs a bounded subscription test with a captured text attachment. Anthropic and Qwen integrations have not been exercised against endpoints.

## Implementation status

This is a developing implementation of [the design](DESIGN.md). The checked application owns project admission, idempotent submissions, model settings, turn sealing, action admission, fan-out receipts and review, final publication, and improvement-session creation. The source for the language changes is pinned in Cargo.lock. The Rust host runs hostless user tool modules in a fresh bounded invocation. The initial user interface provides project navigation, captured attachments, request controls, models, a dictionary, and a snapshot tree.

The full tool catalog and acceptance suite are outstanding. In particular, this version has limited draft qualification, review conditions, dictionary and skill workflows, effect recovery, Git transport, live cache invalidation, and graph layers. A passed scripted scenario establishes the tested path only. See the tests and the published commit for the exact implementation.
