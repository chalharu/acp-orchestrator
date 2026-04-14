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
- `docs/explanation/cli-feedback-first-mvp.md`

The architecture document under `docs/explanation/acp-web-cli-architecture.md`
explains the long-term shape of the stack.
The feedback-first CLI document captures why the repo keeps a minimal
user-visible slice working throughout development.

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

This starts the ACP mock, the web backend, and the CLI frontend as child
processes, then hands terminal I/O to the CLI frontend. Type a prompt, wait for
the streamed assistant reply, and leave the REPL with `/quit`.

When running against the bundled mock stack, prompts containing the word
`permission` trigger a mock permission request.
Use that prompt to exercise `/approve`, `/deny`, and `/cancel`.

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
cargo run -p acp-cli -- chat --new --server-url http://127.0.0.1:8080
```
