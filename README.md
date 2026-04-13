# ACP Orchestrator

ACP Orchestrator is the new home for the ACP-focused orchestration layer.
It sits in front of `copilot --acp --port <port>` and serves shared backend
contracts to Web and CLI clients.

This repository extracts ACP-specific design and implementation work that had
started to outgrow `copilot-sandbox-container`.

## Current focus

- establish the target architecture for the ACP-backed backend / Web / CLI stack
- define the session orchestration and worker supervision model
- grow implementation work in a repository scoped to the orchestrator itself
- ship the feedback-first backend + CLI reference implementation that keeps feedback loops short

## Documents

- `docs/README.md`
- `docs/explanation/acp-web-cli-architecture.md`
- `docs/explanation/cli-feedback-first-mvp.md`

The repository is currently design-first. The architecture document under
`docs/explanation/acp-web-cli-architecture.md` is the primary starting point.
The feedback-first CLI document explains how to ship the first user-visible CLI
slice before the full Ratatui frontend is ready.

## Current workspace layout

- `cargo run` launches the feedback-first stack from the repo root
- `crates/acp-cli` provides the CLI frontend
- `crates/acp-web-backend` provides the HTTP + SSE backend
- `crates/acp-mock` provides the ACP mock service
- `crates/acp-contracts` holds the shared wire contracts

## Quick start

Run the full local stack directly from the repository root:

```bash
cargo run
```

This starts the ACP mock, the web backend, and the CLI frontend as child
processes, then hands terminal I/O to the CLI frontend. Type a prompt, wait for
the streamed assistant reply, and leave the REPL with `/quit`.

## Run each component directly

Start the ACP mock:

```bash
cargo run -p acp-mock -- --port 8090
```

Start the web backend:

```bash
cargo run -p acp-web-backend -- --port 8080 --mock-url http://127.0.0.1:8090
```

Run the CLI frontend against that backend:

```bash
cargo run -p acp-cli -- chat --new --server-url http://127.0.0.1:8080
```
