# ACP Runtime Capabilities: next slice

## 目的

次の実装 slice では、session を「会話ログを持つ HTTP/SSE リソース」から、
agent process と tool execution capability を持つ runtime に引き上げる。
対象は Agent Client Protocol tool calls、filesystem、terminal、Chroot / Docker / K8s
launch の最小設計です。

## Runtime model

- session start 時に backend が agent を 1 つ作成する
- agent は session ID と checkout root に束縛される
- close / transient disconnect 後も、durable metadata と event log から resumable にする
- Web / CLI は引き続き HTTP + SSE の client であり、agent process を直接起動しない

session resume は「同じ session metadata を読み、必要なら agent runtime を再作成する」
操作とする。resume できない場合も transcript は読める状態を維持し、write operation は
明示的な unavailable error にする。

## Agent Client Protocol tool calls

backend は ACP worker と client UI の間で tool call を仲介する。

- tool request / result は session event log に append する
- permission が必要な tool call は pending permission として公開する
- approval / denial は既存の session live ops と同じ owner check を通す
- tool execution は Chroot、Docker container、K8s Pod のいずれかの agent 境界内で実行する
- tool result には secret や host absolute path を含めない

最初の対象 tool は filesystem と terminal に限定し、外部 network / secret manager は別 slice にする。

## Filesystem capability

filesystem access は session checkout root に閉じ込める。

- backend が checkout root を dirfd / validated runtime handle として保持する
- file read/write/list は checkout root からの相対 path だけを受け付ける
- `.git` への direct write、special file、symlink escape、cross-session path は拒否する
- durable metadata には absolute path ではなく checkout relative path だけを保持する

この capability は Web / CLI から直接 exposed せず、agent tool call の実行境界として扱う。

## Terminals

terminal は session checkout root 内で起動する runtime capability とする。

- terminal process は session / user / workspace owner check の後にだけ作成する
- stdin/stdout/stderr は event stream と bounded buffers で扱う
- idle timeout、max lifetime、max concurrent terminal 数を設定可能にする
- close / delete / runtime crash 時には terminal process を best-effort kill する

terminal は host shell に直接出さない。Chroot、Docker container、K8s Pod のいずれかに
閉じ込める構成を steady state とする。

## Configurable agent launch

agent launch command は backend config で定義する。

```text
agent:
  launch:
    mode: chroot | docker | k8s
    command: ["agent-binary", "--stdio"]
    env_allowlist: [...]
    timeout_seconds: 30
```

command は shell string ではなく argv 配列にする。workspace/session IDs、checkout path、
ACP endpoint などは backend が管理する structured env または args として注入する。

## Chroot launch

Chroot mode は local development と最小 self-hosted deployment の最初の target とする。

- session ごとに chroot root を作る
- checkout は chroot root 内に配置または bind mount する
- agent process は non-root user で起動する
- 必要な binary / library / CA bundle だけを chroot root に含める
- filesystem tool と terminal は同じ chroot 境界内で起動する

Chroot mode は container runtime を要求しない代わりに、namespace / cgroup / network 制限は
host 側の追加設定に依存する。multi-tenant deployment では Docker または K8s mode を優先する。

### 現在の最小実装

backend は任意設定の chroot agent process launch を持つ。未設定時は従来どおり mock / external
ACP server への prompt transport だけを使い、agent process は起動しない。

```text
--agent-launch-mode chroot
--agent-command <program>
--agent-command-arg <arg>        # repeatable
--agent-env-allowlist <NAME>     # repeatable
--agent-launch-timeout-seconds <seconds>
--agent-run-uid <uid>
--agent-run-gid <gid>
```

command は shell string ではなく argv として扱われる。現在の chroot launch は Linux と
macOS backend を対象とする。Linux では `cgroup.kill` を持つ writable な cgroup v2 hierarchy
`/sys/fs/cgroup/acp-orchestrator` を使って session ごとの process lifetime を管理する。
chroot mode では checkout は `state_dir/agent-runtimes/<session_id>/root/workspace`
に配置される。agent には `ACP_SESSION_ID`、`ACP_WORKSPACE_ID`、
`ACP_CHECKOUT_ROOT=/workspace`、`ACP_CHECKOUT_RELPATH`、
`ACP_AGENT_LAUNCH_MODE=chroot` を注入する。環境変数は clear される。明示 allowlist
と structured env だけが渡される。agent process は独立 process group として起動する。
checkout tree を agent の non-root uid/gid に割り当てる。cgroup へ参加してから
`PR_SET_NO_NEW_PRIVS` を設定し、chroot / uid/gid drop を行う。close / delete / rollback
では cgroup kill と process group kill の best-effort cleanup を行う。resume 時に runtime
を再作成できない場合でも transcript read は返す。checkout metadata が欠落・session
と不一致の場合も transcript read は返す。write operation は `session runtime unavailable`
として拒否する。

### Admin ACP profiles and dynamic ACP endpoints

backend は state directory 内に global ACP profile を file-backed JSON として保存できる。
profile の一覧取得は authenticated user が利用でき、作成・編集は admin account のみが行う。
Workspaces page header の ACP settings から admin は Claude、Copilot CLI、OpenCode など任意の
一意な名前の ACP profile を追加できる。command は通常の single command line として入力し、frontend
が quote / escape を解釈して argv に分解する。backend は shell を介さず argv を直接実行する。
New Chat modal では既定の mock/static ACP transport か、設定済み profile を選択する。profile
未選択時は従来どおり bundled/static `--acp-server` を使う。既存の global CLI
`--agent-launch-mode chroot ...` 設定も引き続き default launch として動作する。

agent command に ACP placeholder がない場合、backend は stdio ACP subprocess として
実行する。OpenCode の標準 ACP command はこの形で、profile の launch command は
`opencode acp` とする。

Copilot CLI など session ごとの ACP listener port が必要な agent では、argv に
`${ACP_PORT}`、`${ACP_ENDPOINT}`、`${ACP_BASE_URL}`、`${ACP_HOST}` を含める。
backend は launch ごとに localhost port を割り当て、shell を介さず argv 文字列内の
placeholder だけを置換する。あわせて `ACP_HOST=127.0.0.1`、`ACP_PORT`、
`ACP_ENDPOINT=127.0.0.1:<port>`、`ACP_BASE_URL=http://127.0.0.1:<port>` を structured env
として注入する。process spawn 後は launch timeout 内に TCP connect できるまで待ってから
session startup を成功扱いにする。成功時の launch metadata は reply provider に渡され、
prime / request は per-session ACP address を優先し、metadata がない場合だけ default
mock/static address を使う。

OpenCode と local llama.cpp を使う典型 flow は次のとおり。

1. <https://github.com/PrismML-Eng/llama.cpp> を取得して llama-server を build する。
2. Ternary-Bonsai-8B-Q2_0.gguf を Hugging Face から取得する。
3. llama-server --model Ternary-Bonsai-8B-Q2_0.gguf -c 4096 --port `LLAMA_PORT` --host 0.0.0.0
4. ~/.config/opencode/opencode.json の baseURL を `http://HOST:LLAMA_PORT/v1` にする。
5. cargo run -- --web で ACP orchestrator を起動する。
6. ACP settings で profile name に `OpenCode ACP`、ACP launch command に次を保存する。

   ```text
   opencode acp
   ```

7. New Chat で OpenCode ACP profile を選択する。

profile が chroot mode の場合、checkout は session ごとの
`state_dir/agent-runtimes/<session_id>/root/workspace` に作られる。profile 未選択かつ chroot
default launch でない場合は従来の `state_dir/session-checkouts/<session_id>` を使う。
agent command は chroot 後に解決されるため、選択した agent binary と必要な library は chroot
内から見える path に置く必要がある。選択した profile ID は session metadata に保存される。
backend restart 後の restore では保存済み profile ID から command を再解決し、profile が
削除済みの場合は transcript read のみ許可して write operation を拒否する。
cleanup は保存された checkout relative path が同じ session の standard/chroot いずれかの
layout と一致する場合だけ実行し、cross-session path の削除を避ける。

### macOS chroot caveats

macOS backend では chroot mode の最小 process setup として `setsid`、`chroot`、`chdir`、
`setgroups`、`setgid`、`setuid` を行う。Linux 固有の cgroup v2 と `PR_SET_NO_NEW_PRIVS` は
使わず、process group kill と direct child cleanup に依存する。このため lifecycle isolation
と runaway process cleanup は Linux cgroup 構成より弱い。agent が別 session/process group
へ逃がした descendant process の kill-all は保証しない。macOS chroot は開発用途の最小対応
とし、強い isolation が必要な deployment では Linux cgroup、Docker、K8s を使う。
non-Linux/non-macOS backend では chroot launch は unsupported とする。

#### エンドユーザー確認

この sprint で確認できる範囲は agent process の起動・環境注入・per-session ACP endpoint
binding・cleanup までです。profile 未選択時は従来どおり mock / external ACP server を使う。
profile 選択時は起動した ACP endpoint を session の prompt transport として使う。

未設定時は従来動作の確認になる。backend を agent launch flag なしで起動し、Web UI または
session API から session を作成する。通常どおり transcript read/write ができ、state dir に
`agent-runtimes/<session_id>` が作られなければ default path は維持されている。

chroot launch は Linux backend と writable な `/sys/fs/cgroup/acp-orchestrator` を必要とする。
agent command は chroot 後に解決されるため、実行ファイルは chroot 内から見える path に置く。
最小確認では checkout に static な probe binary を入れ、`--agent-command /workspace/<probe>`
として起動する。probe は structured env を `/workspace` 配下の marker file に書く。
対象は `ACP_SESSION_ID`、`ACP_WORKSPACE_ID`、`ACP_CHECKOUT_ROOT`、
`ACP_CHECKOUT_RELPATH`、`ACP_AGENT_LAUNCH_MODE` です。書き込み後は短時間 sleep するだけです。

session 作成後、host 側では
`state_dir/agent-runtimes/<session_id>/root/workspace` に checkout と marker file があることを確認する。
marker file には `ACP_CHECKOUT_ROOT=/workspace` と `ACP_AGENT_LAUNCH_MODE=chroot` が入る。
また `/sys/fs/cgroup/acp-orchestrator/<session_id>/cgroup.procs` に process が入ることを確認する。
close / delete 後は process と runtime directory が best-effort で消える。

## Docker launch

Docker mode は single-node deployment と container-based isolation の target とする。

- session ごとに container を作る
- checkout は bind mount または named volume として read/write mount する
- container user は non-root
- network、capabilities、mounts は最小化する
- terminal は同じ container namespace 内で起動する

Docker mode でも host state directory と他 session checkout は mount しない。

## K8s launch

K8s mode は multi-tenant / scalable deployment の target とする。

- session ごとに Pod または tightly-scoped Job/Pod pair を作る
- owner/session labels を付け、backend が lifecycle を reconcile する
- Pod service account は最小権限にする
- logs/events は backend 経由で session event log に反映する
- Pod crash は session status と restartability deadline に反映する

backend restart 後は durable session metadata と K8s labels から runtime を再発見する。

## Session checkouts on K8s PVC

K8s では checkout を session-scoped PVC に置く。

- PVC 名は session ID から導出し、owner information は label に置く
- close では agent/terminal Pod を止め、PVC は retention policy に従って削除または保持する
- delete では PVC 削除を即時要求し、失敗時は janitor retry 対象にする
- retained transcript と PVC lifetime は別々に管理する

PVC cleanup は idempotent にし、finalizer / janitor で途中失敗から復旧できるようにする。

## 実装順序

1. agent launch config と Chroot launch の最小実装
2. ACP tool call event model と permission roundtrip
3. checkout-root bounded filesystem tools
4. Chroot-confined terminal
5. Docker launch と Docker-confined filesystem / terminal
6. K8s Pod launch
7. session PVC lifecycle と janitor
8. K8s-confined filesystem / terminal

この順序なら、Chroot で runtime contract を固めてから Docker isolation、K8s 固有の
lifecycle と PVC cleanup に進める。
