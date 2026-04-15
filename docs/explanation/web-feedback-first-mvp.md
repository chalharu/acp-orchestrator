# Web Feedback-First MVP 設計

## 1. この文書の位置づけ

`docs/explanation/acp-web-cli-architecture.md` では、最終的な Web フロントエンドを
Leptos CSR ベースの pane 構成と仮想スクロールを備えた UI として定義しています。一方で、
アジャイル開発でエンドユーザーのフィードバックを早く得るには、最終 UI を待たずに
**薄いが実用になる Web フロントエンド**を先に出す方が進めやすいです。

この文書では、最終目標を変えずに、最初に利用者へ見せる **feedback-first な最小 Web**
を定義します。ここでいう最小 Web は「後で捨てる試作品」ではなく、browser 起動導線、
backend contract、session route、permission 操作を先に固めるための**最初の縦スライス**
です。

## 2. 開発前提

- 開発方法はアジャイルとする
- 各スライスの完了時点で、エンドユーザーが実際に触れてフィードバックできる状態を保つ
- Web / CLI ともに backend contract を共有する方針は維持する
- 開発初期のエンドユーザー確認は install 済み frontend ではなく、repo root から
  `cargo run -- --web` を起点に行う
- `cargo run -- --web` は bundled mock と Web backend を起動または再利用し、browser を
  開く launcher とする
- browser の自動 open に失敗した場合は URL を明示表示し、黙って失敗しない
- 最小 Web でも session / prompt / event stream / permission 応答の主要導線は通す
- recent session は backend 上の全件一覧ではなく、この browser から作成または attach した
  session への shortcut を基本とする

## 3. ざっくりしたタスク分割

| スライス | 目的 | 主な作業 | この時点でユーザーが確認できること |
| --- | --- | --- | --- |
| 0. 起動導線固定 | とにかく試せる入口を最短で作る | `cargo run -- --web` を追加し、bundled mock / backend の起動・ヘルス確認・browser open・URL 表示を実装する | repo root から `cargo run -- --web` を実行すると browser が開き、Web frontend の入口へ到達できること |
| 1. 最小会話導線 | まず会話できる状態を早く出す | 単一カラムの chat page、初回 prompt 送信時の session 作成、SSE 受信、transcript 表示、basic composer を実装する | browser から 1 往復以上の会話ができること |
| 2. permission / cancel | 実運用に近い最小制御を足す | permission card、approve / deny button、実行中 turn の cancel action、status 表示を実装する | bundled mock の `verify permission` / `verify cancel` で、permission 応答と cancel を人手で確認できること |
| 3. session 継続 | reload や再訪時のストレスを減らす | session route、history 取得、browser storage の recent-session shortcut、再 attach 導線を実装する | browser を reload または再 open しても既存 session に戻れること |
| 4. 操作性 | 日常利用の不満を減らす | slash command palette、connection badge、tool activity 表示、error banner を実装する | command 候補や接続状態を見ながら操作できること |
| 5. pane / virtual scroll 化 | target architecture に寄せる | recent session pane、tool panel、仮想スクロール、pane layout を実装する | target architecture にある Web MVP の画面構成を確認できること |

### 3.1 フィードバックの取り方

各スライスでは「動くか」だけでなく、次の観点を優先して聞きます。

1. `cargo run -- --web` から会話開始まで迷いが少ないか
2. browser が開いたあと、何をすればよいか直感的に分かるか
3. permission 応答と cancel の場所が見つけやすいか
4. reload / 再訪時の session 継続が期待通りか
5. pane 化や仮想スクロールへ進む前に残る不満が何か

## 4. 最初に見せる最小 Web の設計

### 4.1 形

最初に出す Web は、target architecture にある pane 分割 UI ではなく、**1 カラムの
single-page chat** にします。ただし実装技術は最初から Leptos CSR を使い、
後続で app shell を育てる前提にします。理由は次の通りです。

- `cargo run -- --web` の launcher 体験を最速で end-to-end 接続できる
- browser での session 作成、prompt 送信、SSE 受信、permission 操作を最短で確認できる
- pane や仮想スクロールより先に、route と session 継続の意味を固定できる
- 後続の full Web UI でも使う component / contract の語彙を先に固められる

最終的な Web MVP は pane 構成と仮想スクロールへ進めますが、最初のユーザー確認では
「repo root から迷わず browser を開き、会話を始められるか」を優先します。

### 4.2 初期の起動方法

初期段階では、frontend の個別 build / serve 手順よりも、**repo を clone した直後に
ユーザーがそのまま試せること**を優先します。そのため、root の `cargo run -- --web` を
簡易 launcher とし、workspace 内の frontend / backend / mock を起動または再利用して、
browser を Web app へ開きます。

- Web 起動: `cargo run -- --web`
- browser open 先の例: `http://127.0.0.1:8080/app/`
- browser open に失敗した場合: terminal に URL を表示し、手動 open を案内する

launcher は backend の `/healthz` と asset 配信準備を確認してから browser を開きます。
bundled mock を使う手動確認では、launcher と Web 画面の両方で次の prompt を標準トリガとして
案内します。

- `verify permission`: permission card の表示と approve / deny の確認に使う
- `verify cancel`: 遅延付きの mock reply を始め、cancel action の確認に使う

### 4.3 route と UI 面

最小 Web で先に固定したい surface は次です。

| route / UI | 役割 | 対応する backend contract |
| --- | --- | --- |
| `/app/` | browser 起動直後の入口。composer は見せるが、session は初回送信時に作る | 初回 prompt 時に `POST /api/v1/sessions` |
| `/app/sessions/{id}` | 既存 session に attach して会話を継続する | `GET /api/v1/sessions/{id}` + `GET /api/v1/sessions/{id}/history` + `GET /api/v1/sessions/{id}/events` |
| composer send | prompt を送信する | `POST /api/v1/sessions/{id}/messages` |
| permission card | pending permission に応答する | `POST /api/v1/sessions/{id}/permissions/{requestId}` |
| cancel action | 実行中 turn を止める | `POST /api/v1/sessions/{id}/cancel` |
| close action | session を明示的に終了する | `POST /api/v1/sessions/{id}/close` |

`/app/` では最初から空の chat shell を見せ、利用者が最初の prompt を送った瞬間に session を
作成して `/app/sessions/{id}` へ遷移する想定にします。これにより、browser を開いた直後の
迷いを減らしつつ、未使用 session の量産も避けやすくします。

slice 3 以降の bundled feedback flow では、browser storage に recent session shortcut を
保持します。再訪時は `/app/` から recent session を選ぶか、既知の route
`/app/sessions/{id}` を開いて復帰します。

### 4.4 最小 UX

- browser が開いたら、接続先 backend と session 状態がすぐ分かる
- transcript は時系列に 1 本で表示する
- user / assistant / tool / status / permission を視覚的に区別する
- permission request が来たら approve / deny を即時に押せる card を出す
- stream 切断時は黙ってごまかさず、banner と再 attach 導線を出す
- 初期版では follow mode 固定とし、複雑な pane 分割や仮想スクロールは持ち込まない

表示イメージは次の通りです。

```text
$ cargo run -- --web
opening browser: http://127.0.0.1:8080/app/

+--------------------------------------------------+
| ACP Web MVP                    connected : ready |
+--------------------------------------------------+
| README の要点を教えて                                  |
+--------------------------------------------------+
| [user] README の要点を教えて                           |
| [assistant] ACP Orchestrator は ...                  |
| [permission req_17] read_text_file README.md        |
| [Approve] [Deny]                                    |
+--------------------------------------------------+
```

### 4.5 最初のスコープに入れるもの / 入れないもの

### 入れるもの

- `cargo run -- --web` による bundled mock / backend 起動と browser open
- 初回 prompt 時の session 作成
- 既存 session route への再 attach
- prompt 送信
- live event の購読
- permission request への応答
- 明示的な cancel / close

### 入れないもの

- pane 分割の app shell
- 仮想スクロール
- rich markdown / HTML rendering
- install/package 配布導線
- backend 全件 session 一覧
- session 共有や operator override
- offline 対応や service worker

### 4.6 この設計で避けたいこと

- backend を飛ばして browser から ACP に直接つなぐこと
- `cargo run -- --web` と `/app/*` の責務を分けず、別々の暫定起動方法を増やすこと
- pending permission を transcript の奥に埋めて見落としやすくすること
- browser open や stream reconnect の失敗を黙って隠すこと
- backend 上の session 全件一覧を先に出し、browser-local shortcut 方針を崩すこと

## 5. 次に full Web UI へ進むための接続点

最小 Web の実装で先に固定しておくべきものは次です。

1. `cargo run -- --web` の launcher semantics と app URL
2. `/app/` と `/app/sessions/{id}` の route 語彙
3. transcript に流す event の表示カテゴリ
4. permission request / cancel / close の UI action 語彙
5. recent-session shortcut を browser-local に持つ前提

これらを先に固めれば、後続の Web 実装は「pane layout と仮想スクロールの強化」に集中でき、
会話導線や session 継続の意味を作り直さずに済みます。
