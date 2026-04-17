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

アジャイルで各スライスごとに利用者へ触ってもらい、Web / CLI で shared backend contract
を維持する前提は `docs/explanation/cli-feedback-first-mvp.md` と共通です。この文書では、
そのうえで Web 固有で早めに固定したい前提だけを書きます。

- 開発初期のエンドユーザー確認は install 済み frontend ではなく、repo root から
  `cargo run -- --web` を起点に行う
- `cargo run -- --web` は bundled mock と Web backend を起動または再利用し、browser を
  開く launcher とする
- bundled mock / backend は loopback (`127.0.0.1`) bind を前提とし、launcher が再利用するのも
  loopback 上の healthy instance だけにする
- browser の自動 open に失敗した場合は URL を明示表示し、黙って失敗しない
- 最小 Web でも session / prompt / event stream / permission 応答の主要導線は通す
- Web の認証 transport は architecture doc に合わせて same-origin `Secure` + `HttpOnly`
  cookie を基本とする
- session owner check と state-mutating `POST` の CSRF 保護も slice 0 から崩さない
- session ID は認可や access token の代わりに使わない
- session 一覧や recent ordering の正本は backend が owner 単位で管理する

## 3. 現在の実装状況

このブランチの Web Feedback-First MVP は、当初の「最小 Web」から少し進み、
**launcher + minimal chat shell + permission/cancel + session 再開の一部 + 仮想スクロール**
まで到達しています。現時点の実装をまとめると次の通りです。

| 状態 | 項目 | 現在の実装 | ユーザーが確認できること |
| --- | --- | --- | --- |
| 実装済み | 起動導線 | `cargo run -- --web` が frontend bundle の missing / stale を検知し、必要なら `trunk build --release` を走らせたうえで bundled backend / mock を loopback で起動または再利用し、`/app/` が ready になってから browser を開く | repo root から `cargo run -- --web` を実行すると browser が開き、Web frontend の入口へ安全に到達できること |
| 実装済み | 会話導線 | `/app/` で session を事前作成して `/app/sessions/{id}` へ即時遷移し、startup hint を先に見せたうえで composer / SSE transcript / assistant reply を扱う | browser から startup hint を見て、そのまま 1 往復以上の会話ができること |
| 実装済み | permission / cancel | 固定の permission panel、approve / deny、permission panel 側 cancel、返信待ち中の composer 側 cancel を実装 | bundled mock の `verify permission` / `verify cancel` を人手で確認できること |
| 実装済み | session 再開 | `/app/sessions/{id}` の deep link / reload で既存 session を開ける。さらに同一 tab では `sessionStorage` で「まだ最初の user prompt を送っていない prepared session」を再利用する | reload や direct route でも既存 session に戻れること |
| 実装済み | transcript | chat body は固定高の仮想スクロール viewport になっており、最新発言への追従、safe Markdown rendering、chat 領域外スクロールの抑制が入っている | 長い会話でも transcript だけをスクロールし、最新発言と Markdown 表示を確認できること |
| 実装済み | session list UI | session route shell に collapsible な左 sidebar を追加し、owner-scoped session list を backend の並び順そのままで表示する | 現在の session と closed session を sidebar で見分けながら、別 session へ移動できること |
| 実装済み | close action | sidebar から active session を close API で閉じられる。closed session は list に残り、read-only のまま再訪できる | browser UI が hard-delete を装わず、backend contract 通りの「close 済み session の再表示」を維持していること |
| 未実装 | slash / tool panel | backend には slash completions API があるが、browser UI には command palette / tool activity panel をまだ出していない | 現在の Web は session shell を広げつつも、操作面はまだ絞られていること |

### 3.1 フィードバックの取り方

現状実装では、次の観点を優先してフィードバックを得るのがよいです。

1. `cargo run -- --web` から startup hint 表示まで迷いが少ないか
2. `/app/` が「dashboard」ではなく「session bootstrap route」であることが直感的か
3. permission 応答と cancel の場所が見つけやすいか
4. reload / direct route 再訪時の session 継続が期待通りか
5. transcript の追従・仮想スクロール・Markdown 表示に不満がないか

## 4. 現在の Web MVP の形

### 4.1 形

現在の Web は、target architecture にある最終 multi-pane UI まではまだ到達していませんが、
**chat shell + collapsible left sidebar** までは入っています。ただし実装技術は最初から
Leptos CSR で、backend が shell document と WASM bundle を配信します。

現状の UI は次の 4 面で構成されます。

- `session-sidebar`: `New session` entry point、owned session list、session close affordance
- `chat-topbar`: sidebar toggle と error banner
- `chat-body`: 仮想スクロール付き transcript
- `chat-dock`: pending permission panel と composer

接続状態や session 状態は badge として常時表示するのではなく、composer の status text や
banner に寄せています。session 一覧については narrow screen でも壊れないよう、toggle で
開閉するだけの単純な sidebar に留めています。

### 4.2 起動とビルド

現行コードの起動フローは次の通りです。

- Web 起動: `cargo run -- --web`
- browser open 先の例: `https://127.0.0.1:8443/app/`
- bind / reuse 対象: loopback (`127.0.0.1`) 上の bundled backend / mock のみ
- browser open に失敗した場合: terminal へ URL を出し、失敗を隠さない

#### Leptos CSR ビルドパイプライン

browser 側の実装は `crates/acp-web-frontend/` の Leptos CSR crate で行います。この crate は
Cargo workspace 外の **trunk プロジェクト**です。

```sh
# frontend 単体の主な確認
cargo fmt --manifest-path crates/acp-web-frontend/Cargo.toml
cargo clippy --manifest-path crates/acp-web-frontend/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path crates/acp-web-frontend/Cargo.toml --lib
cargo check --manifest-path crates/acp-web-frontend/Cargo.toml --target wasm32-unknown-unknown
cd crates/acp-web-frontend && trunk build --release
```

repo root からの `cargo run -- --web` は、`crates/acp-web-frontend/dist/` を確認します。
そこで frontend bundle の missing / stale を検知し、必要なら `trunk build --release` を自動実行します。
そのため、通常の bundled feedback flow では事前に手で `trunk build` しなくても動きます。

この bundle 準備ロジックは launcher 本体から `src/frontend_bundle.rs` へ切り出しました。
ここで dist path 解決、missing / stale 判定、入力ファイルの最終更新時刻の比較、
`trunk build --release` の実行を 1 か所で扱います。

backend は dist ディレクトリ内の fingerprinted asset を runtime で見つけ、
stable alias として次の path から配信します。

- `/app/assets/acp-web-frontend.js`
- `/app/assets/acp-web-frontend_bg.wasm`

外部 backend を使う direct mode では、local 側で `frontend_dist` を準備できません。
その backend が bundle を配信していない場合、WASM asset route は
`503 Service Unavailable` になります。

launcher は `/healthz` だけでなく browser entrypoint (`/app/`) 自体の readiness を待ってから
browser を開きます。same-origin `Secure` + `HttpOnly` cookie と CSRF token bootstrap も
loopback HTTPS の app shell で行います。

bundled mock を使う手動確認 prompt は README の bundled feedback flow と同じ
`verify permission` / `verify cancel` を使います。

### 4.3 route と UI 面

**具体的な API path / auth 要件の正本**は `docs/explanation/acp-web-cli-architecture.md`
の 8.3 節ですが、現在の browser 実装が実際に使っている surface は次です。

#### 4.3.1 route 面

| route / shortcut | 現在の役割 | 現在の browser 実装 | 認可・境界メモ |
| --- | --- | --- | --- |
| `/app/` | browser 起動直後の bootstrap route | 「Preparing chat...」を一瞬表示し、session を事前作成または prepared session を再利用して `/app/sessions/{id}` へ即時遷移する | create は mutating `POST` として CSRF を掛け、owner 判定は backend が持つ |
| `/app/sessions/{id}` | live chat route | snapshot 読み込み、SSE 購読、transcript 表示、permission 応答、cancel、composer を扱う | URL 中の session ID 自体は access token ではなく、snapshot / SSE / action ごとに owner check を再実行する |
| direct route / reload | session 継続 | 既知の `/app/sessions/{id}` を開き直して同じ session を読む | deep link を知っていること自体は認可根拠ではない |

現状の `/app/` は session list を見せる landing page ではありません。**まず session を用意して、
startup hint を visible にしたうえで live chat route へ移す**ための bootstrap route です。

#### 4.3.2 action 面

| UI action | 現在の役割 | 実際に使っている backend API | 認可・境界メモ |
| --- | --- | --- | --- |
| composer send | prompt を送信する | message send API | session owner に限って実行し、mutating `POST` として CSRF を掛ける |
| permission panel | pending permission に応答する | permission resolution API | session owner に限って実行し、transcript とは別の固定 panel で扱う |
| cancel action | 実行中 turn を止める | cancel API | permission panel 側と composer 側の両方に cancel affordance がある |
| `New session` entry | 新しい会話へ移る | `/app/` route 経由で session create API | 既存 session を閉じるのではなく、新規 session bootstrap を始める |
| session list | owned session を行き来する | session list API + session snapshot API | list の recent ordering は backend を正本とし、browser 側で並べ替えない |
| session close | active session を終了する | close API | hard-delete は行わず、closed session は list に残して read-only で見せる |

backend には次の API もありますが、現在の browser UI ではまだ使っていません。

- session history API
- slash completions API

### 4.4 現在の最小 UX

- browser が開くと `/app/` は新しい session を準備し、bundled startup hint を見せるために
  `/app/sessions/{id}` へすぐ遷移する
- session route には collapsible な左 sidebar があり、`New session` と owned session list を出す
- session list は backend が返した recent order をそのまま表示し、current session を見分けられる
- active session では close button を使え、close 後も list に残ったまま read-only session として再訪できる
- transcript は `user` / `assistant` / `status` の 3 種を見た目で分けて表示する
- `user` / `assistant` は safe Markdown で HTML 化し、raw HTML / image / unsafe URL は信頼しない
- `status` は plain text のまま表示する
- transcript は仮想スクロールで、ユーザーが意図的に離れていない限り最新発言へ追従する
- page 全体ではなく transcript viewport だけがスクロールする
- pending permission は固定 panel で approve / deny / cancel できる
- 返信待ち中は composer footer に cancel を出す
- stream や session load の失敗は banner と composer status text に出す
- composer は `Enter` で送信し、`Shift+Enter` で改行する
- closed session は snapshot / transcript の表示だけを行い、SSE を張らない read-only 扱いにする
- action 失敗と connection 失敗は別管理にし、action error は次の action を始めるまで維持する
- connection badge、tool activity panel、full multi-pane shell はまだ出していない

### 4.4.1 内部設計の整理

この minimal UI の内部状態も、当初の magic string / tuple 中心の形から少し整理されています。

- session の lifecycle は `"active"` / `"closed"` / `"loading"` のような文字列ではなく
  `SessionLifecycle` enum で持つ
- pending permission は `(String, String)` ではなく `PendingPermission { request_id, summary }`
  で扱う
- SSE transport / parse は `api.rs`、UI state reduction は `lib.rs` に寄せて責務を分ける
- assistant message / status update で turn state を解放する判定は共通 helper
  `should_release_turn_state` にまとめる
- transcript の Markdown sanitization は危険タグの block list を明示したまま、safe structural
  tag は catch-all で通す

表示イメージは次のような minimal shell です。

```text
$ cargo run -- --web
opening browser: https://127.0.0.1:8443/app/

+----------------------+-------------------------------------------+
| New session          | Sessions                                  |
| Session 90abcdef     | Bundled mock ready.                       |
| Current              | Try `verify permission` or `verify cancel`.|
| [Close]              |                                           |
| Session 12abcd34     | [user] README の要点を教えて                    |
| Closed               | [assistant] ACP Orchestrator は ...           |
+----------------------+-------------------------------------------+
|                      | Permission required                       |
|                      | read_text_file README.md                  |
|                      | [Approve] [Deny] [Cancel]                 |
+----------------------+-------------------------------------------+
|                      | Resolve the request below before sending… |
|                      | [textarea]                                |
|                      | [Send]                                    |
+----------------------+-------------------------------------------+
```

### 4.5 現在のスコープに入っているもの / まだ入っていないもの

### 入っているもの

- `cargo run -- --web` による bundled mock / backend 起動と browser open
- frontend bundle の missing / stale 検知と `trunk build --release` の自動実行
- Web auth transport、session owner check、state-mutating `POST` の CSRF 保護
- startup hint を見せるための事前 session 作成
- 既存 session route の再 attach / reload
- session route shell 内の owner-scoped session list
- browser UI からの session close
- prompt 送信
- live event の購読
- permission request への応答
- 実行中 turn の cancel
- 仮想スクロール付き transcript
- safe Markdown rendering

### まだ入っていないもの

- slash command palette
- connection badge / tool activity panel
- full multi-pane app shell
- session 共有や operator override
- offline 対応や service worker

### 4.6 この実装で崩していないこと

- backend を飛ばして browser から ACP に直接つながない
- session ID を access token のように扱わない
- loopback feedback flow を `0.0.0.0` bind や非 local service の暗黙 reuse に広げない
- `cargo run -- --web` と `/app/*` の責務を分けず、別々の暫定起動方法を増やさない
- pending permission を transcript の奥に埋めて見落としやすくしない
- browser open や session / stream の失敗を黙って隠さない
- prepared session を `sessionStorage` で再利用しても、それを認証根拠にはしない

## 5. 次に full Web UI へ進むための接続点

現在の実装で先に固定できた接続点は次です。

1. `cargo run -- --web` の launcher semantics と app URL
2. `/app/` は bootstrap route、`/app/sessions/{id}` は live chat route という語彙
3. transcript に流す `user` / `assistant` / `status` の表示カテゴリ
4. permission request / cancel の UI action 語彙
5. same-origin cookie + CSRF + owner-check を最初から崩さない前提
6. session list / history / close / slash completions は backend 側にあり、browser UI がまだ追いついていないという分離

これにより、後続の Web 実装は「session list UI・close action・slash 操作・pane layout をどう育てるか」
に集中でき、launcher / auth / route / transcript の意味を作り直さずに済みます。
