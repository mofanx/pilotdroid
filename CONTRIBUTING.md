# Contributing

## 环境

- Rust stable（`rustfmt` + `clippy`）；`cargo build` / `cargo test` / `cargo clippy -- -D warnings` 须在提交前全绿。
- 设备端 agent 构建需要 Android SDK platform jar + `d8`，见 `agent/README.md`（仅开发期依赖，设备端零依赖）。

## 流程

- `main` 只收合并：代码改动走 `feature/*` 分支 + PR，review 通过再合。
- 仓库暂无 LICENSE——**未经所有者拍板不要添加**；`Cargo.toml` 中也不要写 `license`/`publish` 字段。

## 设备测试边界（硬性规定）

> **模拟器只用于协议单测与 API 分界验证；能力矩阵结论必须来自真机。**

- `docs/04-capability-matrix.md` 中的每一行能力结论（`/dev/input` 可写性、SELinux 拦截、注入延迟等）**必须**由 `spike/scripts/` 在零售真机上复跑得出。模拟器默认 adb-root、SELinux/输入栈行为与零售 Enforcing 设备不同，数据不可迁移。
- CI（`.github/workflows/ci.yml`）不挂载 android-emulator-runner 做能力断言；如未来添加，仅限 `pilotdroid-proto` 编解码与 API 级兼容检查。
- 新机型接入：按 `docs/04-capability-matrix.md` §4 流程跑 `spike/scripts/` 并追加矩阵行。

## 文档与决策

- 架构/协议/里程碑见 `docs/`；争议性选型先写 spike 实测，再更新 `docs/adr/`。
- 里程碑验收一律以真机实测为准（`docs/05-roadmap.md`），不接受"本地能跑"作为验收依据。

## 磁盘与 worktree 约定

- 磁盘曾三次占满告警（每个 worktree 的 `target/` 独立编译产物）。**新建 worktree 后必做**（非建议）：

  ```bash
  export CARGO_TARGET_DIR=$HOME/.cargo-target/pilotdroid   # 写进 ~/.bashrc 或 worktree 启用脚本
  ```

  cargo 自带锁可安全并发——多 worktree 共用一份 target 可省数 GB。不要用 `.cargo/config.toml`：config 文件在各 worktree 内部，无法跨 worktree 共享。**worktree 合并完成后立即在其内跑 `cargo clean`**。
- `cargo clean` 只影响编译产物，不会丢源码/worktree 元数据。

## Known limitations（对拍与测试前必读）

- **UiAutomation 单持有者**：我方 agent 与 u2 `AutomatorServiceImpl` 互斥（见 ADR-005）。跑 u2 对拍脚本前必须 `pilotdroid agent stop`，反之亦然。
- **`adb tcpip` 会重启 adbd** 并杀死设备上所有 app_process daemon——WiFi 模式与本 agent 不兼容，daemon 须重部署。
- **摊销口径声明规则**：对拍数据必须注明口径——我方 session 摊销（持久连接）vs u2 per-invocation。bench n≥10 的摊销数字不得直接对比 u2 单发数字而不注明。
- **verify-agent.sh logcat 断言已知竞态**：MIUIInput marker 异步落盘，读取采用 settle 轮询（≤3s）；且 MIUI 高噪日志可超过 800 行/s，`-t` 尾窗口必须给到 5000 行余量（`-t 400` 曾把 marker 挤出窗口造成 flaky）。`logcat -d` 后立即 grep 的写法勿复用。
- **adb forward 规则会残留**：forward 规则由 adb server 持有、客户端断开不自动回收。CLI 路径会清掉自己创建的规则，但手动实验/中断可能留下 `tcp:*` 规则——多设备/多端口排查前先 `adb forward --list`，陈旧规则 `adb forward --remove tcp:NNNN` 清场再测。
- **MIUI 系统 buffer**：`MIUIInput` 标记写 system buffer 而非 main，`logcat` 须带 `-b all`。
