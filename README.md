# ACP Orchestrator

ACP Orchestrator is the new home for the ACP-focused orchestration layer.
It sits in front of `copilot --acp --port <port>` and serves shared backend
contracts to Web and CLI clients.

This repository extracts ACP-specific design and implementation work that had
started to outgrow `copilot-sandbox-container`.

## Current focus

- keep the `cargo run` launcher path ready for quick end-user feedback
- iterate on the CLI / backend / mock services independently inside the workspace
- harden the reference implementation with coverage, linting, and hosted quality checks

## Documents

- `docs/README.md`
- `docs/explanation/acp-web-cli-architecture.md`
- `docs/explanation/user-workspace-session-architecture.md`
- `docs/explanation/cli-feedback-first-mvp.md`
- `docs/explanation/web-feedback-first-mvp.md`

The architecture document under `docs/explanation/acp-web-cli-architecture.md`
explains the long-term shape of the stack.
The workspace hierarchy design document explains how `User`, `Workspace`, and
`Session` ownership, Git upstream registration, per-session clone, and cleanup
fit into that target.
The feedback-first CLI document captures why the repo keeps a minimal
user-visible slice working throughout development.
The matching Web feedback-first document defines the first browser-facing slice
around `cargo run -- --web`.

## Current workspace layout

- `cargo run` launches the feedback-first stack from the repo root
- `crates/acp-cli` provides the CLI frontend
- `crates/acp-web-backend` provides the HTTP + SSE backend
- `crates/acp-mock` provides the ACP mock service
- `crates/acp-contracts` holds the shared wire contracts
- `crates/acp-app-support` holds shared launcher/runtime/test support code

This split keeps the root package focused on the easiest local entrypoint while
letting each service stay testable and runnable on its own.

## Quick start

Run the full local stack directly from the repository root:

```bash
cargo run
```

This starts or reuses the bundled ACP mock and web backend, then hands terminal
I/O to the CLI frontend. On interactive terminals the CLI now opens a
multi-pane terminal UI. It shows a session/command pane, transcript pane, input
composer, and tool/status pane. Use `PageUp` / `PageDown` to switch the
transcript into manual scroll mode. Use `End` to jump back to the live tail,
then leave chat with `/quit`.

Open the browser-facing Web launcher from the same repo root:

```bash
cargo run -- --web
```

This starts or reuses the bundled mock/backend. It prints the loopback HTTPS
app URL and attempts to open `/app/` in your browser. The backend uses a local
development certificate for loopback HTTPS. Your browser or OS may require a
one-time trust or confirmation step before the page loads cleanly.

The current Web slice serves a minimal chat shell. It includes a session
sidebar, transcript, composer, and inline status activity. The first prompt
creates a browser-owned session and moves the URL to `/app/sessions/<id>`.
Direct session routes load saved transcript state and keep receiving live
events over SSE. Pending permission requests surface browser controls in the
chat area. Use **Approve**, **Deny**, or **Cancel** there. The composer
supports `/help` only in the browser, with a small floating suggestion overlay.
Recent slash or connection activity is recorded inside the transcript stream.
Deleting the last remaining session returns to a fresh new-chat view.

When stdin/stdout are not terminals, the CLI keeps the older line-oriented mode
for scripting and pipe-driven tests.

On interactive terminals, type `/` or a partial slash command such as `/ap`,
then press `TAB`. The CLI fetches slash-command candidates from the backend in
the composer. After a permission request appears, `/approve` or `/deny`
followed by `TAB` suggests pending request IDs.

When running against the bundled mock stack, prompts containing the word
`permission` still trigger a mock permission request. For reproducible manual
verification, use the built-in mock prompts below:

- `verify permission`: emits `[permission <request-id>] read_text_file README.md`.
  In the browser, use the chat-area controls. In the CLI, respond with
  `/approve <request-id>` or `/deny <request-id>`.
- `verify cancel`: starts a delayed mock reply. Run `/cancel` before the
  assistant reply arrives and confirm `[status] turn cancelled`. In the browser,
  use the visible **Cancel** button while the turn is pending.

For browser regression coverage, run
`python3 -m unittest discover -s tests/playwright -p 'test_*.py'` against a
running web stack. If Chromium needs a user-space sysroot for shared libraries
or fonts, set `ACP_PLAYWRIGHT_SYSROOT=/path/to/sysroot` first.

The root `cargo run` launcher prints the same hints when it starts the bundled
mock for `chat`.

For slice-3 session continuation, the default launcher keeps the bundled stack
available across repeated `cargo run` invocations. A manual resume flow is:

1. `cargo run`
2. Send a prompt, then exit with `/quit`
3. `cargo run -- session list`
4. `cargo run -- chat --session <id>`

The resumed chat prints the saved transcript before returning to the prompt.

If you already have an ACP server running, point the launcher at it instead of
starting the bundled mock:

```bash
cargo run -- --acp-server 127.0.0.1:8090
```

## Run each component directly

Start the ACP mock:

```bash
cargo run -p acp-mock -- --port 8090
```

Start the web backend:

```bash
cargo run -p acp-web-backend -- --port 8080 --acp-server 127.0.0.1:8090
```

Run the CLI frontend against that backend:

```bash
cargo run -p acp-cli -- chat --new --server-url https://127.0.0.1:8080
```
