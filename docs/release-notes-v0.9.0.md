# pilotdroid v0.9.0 — select_device：会话内设备切换

**头条**：多设备用户不再需要在 MCP 配置里绑定 serial、改配置、重启宿主——
`select_device`/`list_devices` 两个新工具让 AI 在**会话中途**一键切换到任何
adb 设备。注册时的 serial 降级为「初始设备」。

## 新增

### `list_devices` / `select_device`（MCP 工具面 30→32）

- `list_devices`：adb devices 直通 `[{serial,state}]` + 当前目标标记——
  device-free，无需连接。
- `select_device {serial}`：进程内 teardown 当前连接（RAII 摘 forward）并
  重绑新设备。**切换即作废旧设备全部状态**：元素快照/`i` 编号、armed
  watcher、缓冲事件——响应如实报告 `cleared` 清单；connect 失败自动回滚到
  原设备（错误注明 still on X）。切中 stream 会终止流（已文档化）。

### 正确性语义（防跨设备静默错点）

- 索引快照携带**设备 serial 标签**：消费错设备快照报显式
  device-mismatch 错——`i` 编号是 per-device 状态，绝不静默点错手机。
- 正常路径切换即清空快照：切后 `tap_index` 收到「no element snapshot —
  re-observe」显式报错；serial 标签是防御纵深，兜未来遗漏清空的路径。

### `docs/use-from-other-projects.md`

跨项目接入说明同步重写 §4：`select_device` 为推荐换机方式，env pin 降级为
初始设备。

## 验证

- 单测 +4（切换失效/失败回滚/device-free list/错机快照），工作区 **111/0**。
- 真机往返：小米 ba0d072d ↔ 华为平板 CFE6R21525008201——会话中途切换后
  describe 出异机内容、`tap_index` 导航落在新机、切回闭环。
- doc-drift / fmt / clippy 全净，CI+CodeQL 绿。

## 升级

```
pip install -U pilotdroid     # 或 brew upgrade pilotdroid
```

agent.jar 无变更（0.7.4 随行）——纯 host 侧 feature，无设备端重部署需求。
