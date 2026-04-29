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

command は shell string ではなく argv として扱われる。現在の chroot launch は Linux
backend のみを対象とする。`cgroup.kill` を持つ writable な cgroup v2 hierarchy
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
