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
