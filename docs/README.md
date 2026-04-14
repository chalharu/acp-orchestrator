# ドキュメント案内

このページは、`acp-orchestrator` の文書を読む入口です。まず repo の目的を把握し、
次に ACP Orchestrator の target design を読む流れを基本にします。

## まずはここから

| したいこと | 読む文書 |
| --- | --- |
| リポジトリの目的と現在地を知りたい | `README.md` |
| ACP ベースの backend・Web・CLI の目標アーキテクチャを知りたい | `docs/explanation/acp-web-cli-architecture.md` |
| アジャイル前提のタスク分割と、最初に見せる最小 CLI の設計を知りたい | `docs/explanation/cli-feedback-first-mvp.md` |

## ドキュメントの役割

- Overview: `README.md`
  - リポジトリの目的、スコープ、現在の作業軸、ローカル試行手順、bundled mock の
    手動確認 prompt（`verify permission` / `verify cancel`）をつかむ
- Explanation: `docs/explanation/acp-web-cli-architecture.md`
  - ACP Orchestrator の target architecture、責務分離、設計判断を理解する
- Explanation: `docs/explanation/cli-feedback-first-mvp.md`
  - feedback-first な CLI の段階的な出し方と、最初のユーザー確認面を理解する

## 読み進め方のおすすめ

1. 全体像は `README.md`
2. 設計判断と境界は `docs/explanation/acp-web-cli-architecture.md`
3. 最初の CLI をどう刻んで出すかは `docs/explanation/cli-feedback-first-mvp.md`
