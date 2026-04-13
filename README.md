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
- ship the slice 1 backend + CLI reference implementation that keeps feedback loops short

## Documents

- `docs/README.md`
- `docs/explanation/acp-web-cli-architecture.md`
- `docs/explanation/cli-feedback-first-mvp.md`

The repository is currently design-first. The architecture document under
`docs/explanation/acp-web-cli-architecture.md` is the primary starting point.
The feedback-first CLI document explains how to ship the first user-visible CLI
slice before the full Ratatui frontend is ready.

## Slice 1 quick start

Run the first user-visible CLI slice directly from the repository root:

```bash
cargo run --bin acp -- chat --new
```

Type a prompt, wait for the streamed assistant reply, and leave the REPL with
`/quit`.

The current slice uses a mock conversation engine behind an HTTP + SSE backend
so the CLI contract can be exercised before ACP worker integration lands.

If you want a long-running backend for repeated manual experiments, start it in
one terminal:

```bash
cargo run --bin acp -- serve
```

Then point the chat command at it from another terminal:

```bash
cargo run --bin acp -- chat --new --server-url http://127.0.0.1:8080
```
