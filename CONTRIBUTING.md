# Contributing

This document is the source of truth for contribution rules.

## 1. Development Flow

1. Create a branch from `main`.
2. Implement changes and add/update tests.
3. Commit using Conventional Commits.

## 2. Branch Rules

- `main`: always releasable.
- Working branches: `feature/<topic>`, `fix/<topic>`, `chore/<topic>`.
- Direct push to `main` is not allowed.

## 3. Commit Message Rules (Conventional Commits)

Format:

`<type>(<scope>): <subject>`

Examples:

- `feat(api): add user profile endpoint`
- `fix(parser): handle empty input`
- `docs(readme): clarify setup steps`
- `chore(ci): update workflow cache key`

Types:

- `feat`: new feature
- `fix`: bug fix
- `docs`: documentation only
- `refactor`: code change without behavior change
- `test`: tests
- `chore`: maintenance/configuration

## 4. Local and Containerized Validation

- Hosted lint is provided by the external `linter-service`; the repository-managed
  local validation baseline starts at `scripts/build-test.sh`.
- `scripts/build-test.sh` is the supported local build/test entry point.
- `scripts/build-test.sh` uses Docker Buildx by default. `--build-only` can fall
  back to an ephemeral Kubernetes Buildkitd when the Docker daemon is unavailable.
- Use `CONTROL_PLANE_TOOLCHAIN=docker` or `CONTROL_PLANE_TOOLCHAIN=buildkitd` to
  force the build surface explicitly.
- Use trusted upstream images when they already satisfy the contract.
- If only a third-party image exists, build a thin repository-managed image and
  publish it to GHCR for reuse.
- GitHub Actions validation should pass without extra registry secrets.
- Keep the Renovate dry-run scoped to public dependencies and the pinned
  external skills repository.
- `scripts/test-standalone.sh` and `scripts/test-kind.sh` remain the lower-level
  smoke / integration scripts used by `scripts/build-test.sh`.
- When this repository is developed from inside a containerized tooling
  environment, keep these scripts unchanged.
- In that environment, also provide `docker buildx` together with `kubectl`,
  `ssh`, and `ssh-keygen`.
- In that environment, full runtime / Kind coverage still needs a
  Docker-compatible container runtime.
- In that environment, the Buildkitd fallback only covers
  `scripts/build-test.sh --build-only`.
- When behavior or operator guidance changes, keep `README.md`,
  `docs/README.md`, `docs/how-to-guides/cookbook.md`,
  `docs/explanation/knowledge.md`, `docs/reference/control-plane-runtime.md`,
  and `docs/reference/debug-log.md` aligned in the same PR.
