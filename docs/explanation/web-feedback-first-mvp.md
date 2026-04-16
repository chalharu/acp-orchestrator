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

## 3. ざっくりしたタスク分割

| スライス | 目的 | 主な作業 | この時点でユーザーが確認できること |
| --- | --- | --- | --- |
| 0. 起動導線固定 | とにかく試せる入口を最短で作る | `cargo run -- --web` を追加し、bundled mock / backend の loopback 起動・ヘルス確認・browser bootstrap readiness・loopback HTTPS endpoint・browser open・URL 表示と、Web auth / CSRF の足場を実装する | repo root から `cargo run -- --web` を実行すると browser が開き、Web frontend の入口へ安全に到達できること |
| 1. 最小会話導線 | まず会話できる状態を早く出す | 単一カラムの chat page、初回 prompt 送信時の session 作成、SSE 受信、transcript 表示、basic composer を実装する | browser から 1 往復以上の会話ができること |
| 2. permission / cancel | 実運用に近い最小制御を足す | permission card、approve / deny button、実行中 turn の cancel action、status 表示を実装する | bundled mock の `verify permission` / `verify cancel` で、permission 応答と cancel を人手で確認できること |
| 3. session 継続 | reload や再訪時のストレスを減らす | backend-managed session list、session route、history 取得、再 attach 導線を実装する | browser を reload または再 open しても既存 session に戻れること |
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
簡易 launcher とし、workspace 内の Web backend / mock を起動または再利用して、backend が
配信する browser entrypoint を開きます。frontend asset の build / serve も、独立 service として
ではなく backend entrypoint 配下にぶら下げます。

- Web 起動: `cargo run -- --web`
- browser open 先の例: `https://127.0.0.1:8443/app/`
- bundled feedback flow の bind 先: `127.0.0.1` のみ
- browser open に失敗した場合: terminal へ URL を表示し、手動 open を案内する

#### Leptos CSR ビルドパイプライン (slice 1 以降)

Slice 1 から browser 側の実装は `crates/acp-web-frontend/` の Leptos CSR crate で
行います。この crate は Cargo workspace とは独立した **trunk プロジェクト**であり、
通常の `cargo build` や `cargo test` は対象外です。ビルド手順は次の通りです。

```sh
# 1. wasm32 ターゲットを追加（初回のみ）
rustup target add wasm32-unknown-unknown

# 2. trunk を用意（バイナリを https://github.com/trunk-rs/trunk からインストール）
# 例: cargo install trunk --locked
# または prebuilt バイナリを PATH に配置

# 3. frontend をビルド（repo root または crates/acp-web-frontend から実行）
cd crates/acp-web-frontend && trunk build --release

# 4. backend をビルド / 起動
cargo build -p acp-web-backend
```

`trunk build` は `crates/acp-web-frontend/dist/` に JS loader と WASM binary を出力します。
Backend はこの dist ディレクトリを runtime で参照します。
fingerprinted bundle は
`/app/assets/acp-web-frontend.js` と `/app/assets/acp-web-frontend_bg.wasm`
の stable alias から配信します。
dist が存在しない場合、backend は 503 を返します。
ブラウザには「frontend 未ビルド」を示すプレースホルダを表示します。

開発中は `trunk serve` を使い、Axum backend を別プロセスで起動して proxy を通すことで
ホットリロードを実現できます（production での個別 serve は不要です）。

launcher は backend の `/healthz` と asset 配信準備に加えて、Web entrypoint の
browser-bootstrap readiness を確認してから browser を開きます。`/healthz` は process の
liveness/readiness 用であり、それだけで auth cookie / CSRF 初期化まで完了した根拠にはしません。
bundled service を再利用する場合も、loopback 上の起動だけを対象とし、非 local な endpoint へ
暗黙に attach しません。
feedback-first launcher でも browser 入口は loopback HTTPS とし、ここで same-origin
`Secure` + `HttpOnly` cookie と CSRF token を bootstrap します。つまり、local feedback flow でも
session ID を token 代わりにしたり、CSRF を省略したりせず、本番と同じ auth / owner-check
前提で始めます。
この bootstrap の責務は層ごとに分けます。Web backend の entrypoint は loopback HTTPS endpoint、
cert material の提示、auth cookie 発行、CSRF token 初期化を担当します。launcher はその
readiness を待って browser を開きます。local certificate を OS / browser が信頼するための
setup や明示承認フローは host 側の責務として扱います。trust が成立していない場合は launcher が
失敗を隠さず、URL と対処を表示します。

bundled mock を使う手動確認 prompt は README の bundled feedback flow と同じ
`verify permission` / `verify cancel` を使います。

### 4.3 route と UI 面

最小 Web で先に固定したい surface は次です。**具体的な API path / auth 要件の正本**は
`docs/explanation/acp-web-cli-architecture.md` の 8.3 節とします。この文書では、最初の Web
slice がどの API 群を使い、どの trust boundary を守るかを固定します。

#### 4.3.1 route 面

| route / shortcut | 役割 | 利用する backend API 群 | 認可・境界メモ |
| --- | --- | --- | --- |
| `/app/` | browser 起動直後の入口。composer は見せるが、owner-scoped session list も表示する | owned session list API + session create API | loopback HTTPS 上で same-origin cookie principal として開始し、一覧も create も backend の owner 判定に従う |
| `/app/sessions/{id}` | 既存 session を開く。attach 可能なら会話を継続し、closed なら read-only transcript を表示する | session snapshot / history / event stream API | URL 中の session ID 自体は access token ではなく、snapshot / history / SSE attach でも owner check を再実行する |
| session list pane | browser から再訪しやすくする一覧 | owned session list API | session 一覧と recent ordering の正本は backend が持ち、active / closed の state も backend から受け取る |

#### 4.3.2 action 面

| UI action | 役割 | 利用する backend API 群 | 認可・境界メモ |
| --- | --- | --- | --- |
| composer send | prompt を送信する | message send API | session owner に限って実行し、mutating `POST` として CSRF を掛ける |
| fixed permission card | pending permission に応答する | permission resolution API | session owner に限って実行し、transcript には read-only event だけを流す |
| cancel action | 実行中 turn を止める | cancel API | session owner に限って実行し、mutating `POST` として CSRF を掛ける |
| close action | session を明示的に終了する | session close API | session owner に限って実行し、mutating `POST` として CSRF を掛ける |

`/app/` では最初から空の chat shell を見せ、利用者が最初の prompt を送った瞬間に session を
作成して `/app/sessions/{id}` へ遷移する想定にします。これにより、browser を開いた直後の
迷いを減らしつつ、未使用 session の量産も避けやすくします。

`/app/sessions/{id}` は backend の session state に従って開き方を変えます。attach 可能な
session では composer / permission / cancel / close を伴う live view を出します。
retained closed session では transcript と state badge だけを出す read-only view にします。

slice 3 以降の bundled feedback flow では、backend が owner-scoped session list を返します。
再訪時は `/app/` から session 一覧を選ぶか、既知の route `/app/sessions/{id}` を開いて復帰します。
retention window 内にある closed session も state 付きで見せられます。ただし、deep link を知って
いること自体は認可根拠にならず、backend は毎回 owner を確認します。

### 4.4 最小 UX

- browser が開いたら、接続先 backend と session 状態がすぐ分かる
- transcript は時系列に 1 本で表示し、conversation / tool / status の read-only event を流す
- user / assistant / tool / status を視覚的に区別する
- permission request は transcript に埋めず、固定の permission card で approve / deny できるようにする
- stream 切断時は黙ってごまかさず、banner と再 attach 導線を出す
- 初期版では follow mode 固定とし、複雑な pane 分割や仮想スクロールは持ち込まない

表示イメージは次の通りです。

```text
$ cargo run -- --web
opening browser: https://127.0.0.1:8443/app/

+--------------------------------------------------+
| ACP Web MVP                    connected : ready |
+--------------------------------------------------+
| README の要点を教えて                                  |
+--------------------------------------------------+
| [user] README の要点を教えて                           |
| [assistant] ACP Orchestrator は ...                  |
| [status] permission req_17 pending                  |
|                                                    |
| pending permission                                 |
| [permission req_17] read_text_file README.md        |
| [Approve] [Deny]                                    |
+--------------------------------------------------+
```

### 4.5 最初のスコープに入れるもの / 入れないもの

### 入れるもの

- `cargo run -- --web` による bundled mock / backend 起動と browser open
- Web auth transport、session owner check、state-mutating `POST` の CSRF 保護
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
- session ID を access token のように扱うこと
- loopback feedback flow を `0.0.0.0` bind や非 local service の暗黙 reuse に広げること
- `cargo run -- --web` と `/app/*` の責務を分けず、別々の暫定起動方法を増やすこと
- pending permission を transcript の奥に埋めて見落としやすくすること
- browser open や stream reconnect の失敗を黙って隠すこと
- owner-scoped session 一覧を backend ではなく browser-local state だけで管理すること

## 5. 次に full Web UI へ進むための接続点

最小 Web の実装で先に固定しておくべきものは次です。

1. `cargo run -- --web` の launcher semantics と app URL
2. `/app/` と `/app/sessions/{id}` の route 語彙
3. transcript に流す event の表示カテゴリ
4. permission request / cancel / close の UI action 語彙
5. owner-scoped session list を backend が管理する前提
6. shortcut / deep link と認証・認可を切り離す前提

これらを先に固めれば、後続の Web 実装は「pane layout と仮想スクロールの強化」に集中でき、
会話導線や session 継続の意味を作り直さずに済みます。
