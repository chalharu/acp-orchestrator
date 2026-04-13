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

## Documents

- `docs/README.md`
- `docs/explanation/acp-web-cli-architecture.md`

The repository is currently design-first. The architecture document under
`docs/explanation/acp-web-cli-architecture.md` is the primary starting point.
