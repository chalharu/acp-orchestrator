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

## 4. Local and Hosted Validation

- Hosted lint is provided by the external `linter-service` using
  `.github/linter-service.yaml`.
- The supported local validation baseline for this Rust workspace is:
  - `cargo fmt --all`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- If a change is coverage-sensitive or a PR is failing Sonar coverage, mirror the
  hosted coverage job with
  `cargo llvm-cov --workspace -j1 --lcov --output-path coverage/lcov.info -- --test-threads=1`.
- Keep Renovate configuration changes in `renovate.json5`.
- When behavior or contributor guidance changes, keep `README.md`,
  `docs/README.md`, `docs/explanation/acp-web-cli-architecture.md`,
  `docs/explanation/cli-feedback-first-mvp.md`, and
  `docs/explanation/web-feedback-first-mvp.md` aligned in the same PR.
