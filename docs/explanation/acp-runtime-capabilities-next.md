# ACP Runtime Capabilities: next slice

## 目的

次の実装 slice では、session を「会話ログを持つ HTTP/SSE リソース」から、
agent process と tool execution capability を持つ runtime に引き上げる。
対象は Agent Client Protocol tool calls、filesystem、terminal、Docker/Kubernetes
launch の最小設計である。

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

terminal は host shell に直接出さない。Docker または Kubernetes Pod 内に閉じ込める構成を
steady state とする。

## Configurable agent launch

agent launch command は backend config で定義する。

```text
agent:
  launch:
    mode: docker | kubernetes
    command: ["agent-binary", "--stdio"]
    env_allowlist: [...]
    timeout_seconds: 30
```

command は shell string ではなく argv 配列にする。workspace/session IDs、checkout path、
ACP endpoint などは backend が管理する structured env または args として注入する。

## Docker launch

Docker mode は local development と single-node deployment の最初の target とする。

- session ごとに container を作る
- checkout は bind mount または named volume として read/write mount する
- container user は non-root
- network、capabilities、mounts は最小化する
- terminal は同じ container namespace 内で起動する

Docker mode でも host state directory と他 session checkout は mount しない。

## Kubernetes launch

Kubernetes mode は multi-tenant / scalable deployment の target とする。

- session ごとに Pod または tightly-scoped Job/Pod pair を作る
- owner/session labels を付け、backend が lifecycle を reconcile する
- Pod service account は最小権限にする
- logs/events は backend 経由で session event log に反映する
- Pod crash は session status と restartability deadline に反映する

backend restart 後は durable session metadata と Kubernetes labels から runtime を再発見する。

## Session checkouts on Kubernetes PVC

Kubernetes では checkout を session-scoped PVC に置く。

- PVC 名は session ID から導出し、owner information は label に置く
- close では agent/terminal Pod を止め、PVC は retention policy に従って削除または保持する
- delete では PVC 削除を即時要求し、失敗時は janitor retry 対象にする
- retained transcript と PVC lifetime は別々に管理する

PVC cleanup は idempotent にし、finalizer / janitor で途中失敗から復旧できるようにする。

## 実装順序

1. agent launch config と Docker launch の最小実装
2. ACP tool call event model と permission roundtrip
3. checkout-root bounded filesystem tools
4. Docker-confined terminal
5. Kubernetes Pod launch
6. session PVC lifecycle と janitor
7. Kubernetes-confined terminal

この順序なら、local Docker で runtime contract を固めてから Kubernetes 固有の lifecycle と
PVC cleanup に進める。
