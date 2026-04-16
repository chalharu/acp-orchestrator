# CLI Feedback-First MVP 設計

## 1. この文書の位置づけ

`docs/explanation/acp-web-cli-architecture.md` では、最終的な CLI フロントエンドを
Ratatui ベースの multi-pane UI として定義しています。一方で、アジャイル開発で
エンドユーザーのフィードバックを早く得るには、最終 UI を待たずに**薄いが実用になる
CLI**を先に出す方が進めやすいです。

この文書では、最終目標を変えずに、最初に利用者へ見せる **feedback-first な最小 CLI**
を定義します。ここでいう最小 CLI は「後で捨てる試作品」ではなく、backend contract と
static command 定義を先に固めるための**最初の縦スライス**です。

## 2. 開発前提

- 開発方法はアジャイルとする
- 各スライスの完了時点で、エンドユーザーが実際に触れてフィードバックできる状態を保つ
- Web / CLI ともに backend contract を共有する方針は維持する
- 最小 CLI でも session / prompt / event stream / permission 応答の主要導線は通す
- 開発初期のエンドユーザー確認は install 済み CLI ではなく、repo root から `cargo run` を起点に行う

## 3. ざっくりしたタスク分割

| スライス | 目的 | 主な作業 | この時点でユーザーが確認できること |
| --- | --- | --- | --- |
| 0. 契約固定 | CLI を載せる土台を固定する | session DTO、SSE event schema、permission 応答、認証 transport、session owner check を実装する | まだ内部向け。以後の CLI が同じ contract に乗ること |
| 1. 最小会話導線 | とにかく会話できる状態を早く出す | session 作成、prompt 送信、SSE 受信、終了処理、line-oriented CLI を実装する | repo root から `cargo run` を実行し、会話を 1 往復以上できること |
| 2. permission / cancel | 実運用に近い最小制御を足す | permission request 表示、approve / deny、実行中 turn の cancel を実装する | bundled mock の `verify permission` / `verify cancel` で、permission 応答と cancel を人手で確認できること |
| 3. session 継続 | 再利用と途中復帰を可能にする | backend-managed session list / attach、history 取得、snapshot 表示を実装する | CLI を再起動しても既存 session に戻れること |
| 4. 補完と操作性 | 日常利用の不満を減らす | slash command catalog、TAB 補完、基本的な status 表示を実装する | slash command 候補を見ながら操作できること |
| 5. Ratatui 置換 | 最終 CLI MVP へ進む | multi-pane layout、scroll 制御、tool/status pane を実装する | target architecture にある CLI MVP の画面構成を確認できること |

### 3.1 フィードバックの取り方

各スライスでは「動くか」だけでなく、次の観点を優先して聞きます。

1. 会話開始までの迷いが少ないか
2. permission 応答の意味が分かりやすいか
3. session 復帰が期待通りか
4. slash command の名前が自然か
5. Ratatui 化する前に残る不満が何か

## 4. 最初に見せる最小 CLI の設計

### 4.1 形

最初に出す CLI は、Ratatui の multi-pane UI ではなく、**1 画面の line-oriented REPL**
にします。理由は次の通りです。

- backend contract の妥当性を先に確認できる
- transcript / input / permission 応答を最短で end-to-end 接続できる
- 後続の Ratatui 実装でも使う command 名と session 操作を先に固定できる

最終的な CLI MVP は Ratatui へ進めますが、最初のユーザー確認では
「terminal から迷わず会話を始められるか」を優先します。

### 4.2 初期の起動方法

初期段階では、配布済み binary や install 手順の整備よりも、**repo を clone した直後に
ユーザーがそのまま試せること**を優先します。そのため、root の `cargo run` を簡易
launcher とし、workspace 内の frontend / backend / mock を起動して
frontend に I/O を渡します。個別実行が必要な場合のみ `cargo run -p <package>` を
使います。

- 新規会話開始: `cargo run`
- 既存 session へ再 attach: `cargo run -- chat --session <id>`
- owned session の確認: `cargo run -- session list`

slice 3 以降の bundled feedback flow では、launcher が bundled backend / mock を
再利用します。`/quit` 後に `cargo run -- chat --session <id>` を実行しても、同じ
session へ戻れます。

bundled mock を使う手動確認では、launcher が次の prompt を表示上の標準トリガとして案内します。

- `verify permission`: `[permission <request-id>] ...` を出し、`/approve <request-id>` または
  `/deny <request-id>` の確認に使う
- `verify cancel`: 遅延付きの mock reply を始め、`/cancel` の確認に使う

この文書で `acp ...` と書く箇所は、将来の install 後も維持したい**論理コマンド名**です。
開発初期の実行では、それを root launcher 経由の `cargo run` や、個別 component 用の
`cargo run -p ...` に読み替えます。

### 4.3 コマンド面

コマンド名は仮に `acp` とします。

| コマンド | 役割 | 対応する backend contract |
| --- | --- | --- |
| `acp chat --new` | 新規 session を作って会話を開始する | `POST /api/v1/sessions` + `GET /api/v1/sessions/{id}/events` + `POST /api/v1/sessions/{id}/messages` |
| `acp chat --session <id>` | 既存 session を開く。attach 可能なら会話を再開し、closed なら read-only で表示する | `GET /api/v1/sessions/{id}` + `GET /api/v1/sessions/{id}/history` + `GET /api/v1/sessions/{id}/events` |
| `acp session list` | owner session を一覧する | `GET /api/v1/sessions` |
| `acp session close <id>` | session を明示的に終了する | `POST /api/v1/sessions/{id}/close` |

`acp chat` の中では、通常入力を prompt、`/` 始まりの入力を session-local command として扱います。

| REPL command | 役割 |
| --- | --- |
| `/help` | 利用可能な command を表示する |
| `/cancel` | 実行中 turn を cancel する |
| `/approve <request-id>` | permission request を許可する |
| `/deny <request-id>` | permission request を拒否する |
| `/quit` | chat REPL を終了する |

この command 群は最終的な slash command catalog の最小集合でもあります。つまり、
後で TAB 補完や Ratatui composer を実装しても、command の意味は変えません。

`acp session list` の正本は、backend が owner 単位で管理する session 一覧に置きます。
CLI はその一覧と current state を表示して attach / read-only 参照の導線を提供し、
session 管理自体は backend 側へ寄せます。
`acp chat --session <id>` も backend が返す session state に従って動作します。attach 可能な
session では live session を再開します。retained closed session では read-only transcript を
開きます。

### 4.4 最小 UX

- transcript は時系列に 1 本で表示する
- user / assistant / tool / status / permission を文字プレフィックスで区別する
- permission request が来たら request ID と要約を即時表示する
- stream 切断時は自動でごまかさず、原因を表示して明示的に再 attach させる
- 初期版では follow mode 固定とし、複雑な scroll 制御は持ち込まない

表示イメージは次の通りです。

```text
$ cargo run
session: s_01H...
connected to backend: https://127.0.0.1:8080

> README の要点を教えて
[assistant] ACP Orchestrator は ...
[permission req_17] read_text_file README.md

> /approve req_17
[status] permission req_17 approved
[assistant] README の要点は ...

> /quit
```

### 4.5 最初のスコープに入れるもの / 入れないもの

### 入れるもの

- 新規 session の作成
- 既存 session への再 attach
- prompt 送信
- live event の購読
- permission request への応答
- 明示的な close / quit

### 入れないもの

- Ratatui multi-pane layout
- virtual scroll
- TAB 補完
- install/package 配布導線
- 複数 session の同時表示
- session 共有や operator override

### 4.6 この設計で避けたいこと

- 最終 CLI と無関係な一時専用 command を増やすこと
- backend を飛ばして CLI から ACP に直接つなぐこと
- permission や session owner check を「後で付ける前提」で省略すること
- stream 切断を黙って再試行し、状態不整合を見えなくすること

## 5. 次に Ratatui へ進むための接続点

最小 CLI の実装で先に固定しておくべきものは次です。

1. `acp chat` と session command の語彙
2. transcript に流す event の表示カテゴリ
3. permission request の提示形式
4. static slash command catalog の最小集合

これらを先に固めれば、後続の Ratatui 実装は「画面構成の強化」に集中でき、会話導線や
session 制御の意味を作り直さずに済みます。
