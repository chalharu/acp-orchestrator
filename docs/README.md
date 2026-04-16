# ドキュメント案内

このページは、`acp-orchestrator` の文書を読む入口です。まず repo の目的を把握し、
次に ACP Orchestrator の target design を読む流れを基本にします。

## まずはここから

| したいこと | 読む文書 |
| --- | --- |
| リポジトリの目的と現在地を知りたい | `README.md` |
| ACP ベースの backend・Web・CLI の目標アーキテクチャを知りたい | `docs/explanation/acp-web-cli-architecture.md` |
| 最小 CLI の出し方と設計を知りたい | `docs/explanation/cli-feedback-first-mvp.md` |
| 最小 Web の出し方と設計を知りたい | `docs/explanation/web-feedback-first-mvp.md` |

## ドキュメントの役割

- Overview: `README.md`
  - リポジトリの目的、スコープ、現在の作業軸、ローカル試行手順をつかむ
  - bundled mock の手動確認 prompt（`verify permission` / `verify cancel`）、
    slice 0 の `cargo run -- --web` browser launcher、slice 5 の multi-pane terminal
    UI、slash command 補完、`session list` / `chat --session` による session 復帰手順も
    ここで確認する
- Explanation: `docs/explanation/acp-web-cli-architecture.md`
  - ACP Orchestrator の target architecture、責務分離、設計判断を理解する
- Explanation: `docs/explanation/cli-feedback-first-mvp.md`
  - feedback-first な CLI の段階的な出し方と、最初のユーザー確認面を理解する
- Explanation: `docs/explanation/web-feedback-first-mvp.md`
  - feedback-first な Web の段階的な出し方と、`cargo run -- --web` を軸にした最初の
    ユーザー確認面を理解する

## 読み進め方のおすすめ

1. 全体像は `README.md`
2. 設計判断と境界は `docs/explanation/acp-web-cli-architecture.md`
3. 最初の CLI をどう刻んで出すかは `docs/explanation/cli-feedback-first-mvp.md`
4. 最初の Web をどう刻んで出すかは `docs/explanation/web-feedback-first-mvp.md`
