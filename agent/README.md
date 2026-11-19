# agent/

设备端 agent（Java dex，`app_process` 常驻）。设计见 `docs/03-device-agent.md`。

## 状态

骨架：**HELLO + 注入（TAP/inject）已实现**（复用 spike 验证过的 `InputManagerGlobal` 反射路径），DUMP/QUERY/SCREENSHOT 留 TODO——刻意等 spike-2 对「resident UiAutomation 获取方式与延迟」的裁决后再写。

传输层目前是**占位行协议**（`localabstract:pilotdroid` 上的行命令），M2 换成 `docs/02-protocol.md` 的二进制分帧。

## 构建与运行（需 Android SDK build-tools，仅开发期依赖）

```bash
# 编译（android.jar 来自 SDK platforms，与 spike/scripts/05 相同方法）
javac --release 8 -classpath $ANDROID_HOME/platforms/android-34/android.jar \
    -d build agent/java/pilotdroid/Agent.java
d8 --min-api 26 --output agent.jar build/pilotdroid/Agent*.class   # dex 化

# 部署 + 拉起
adb push agent.jar /data/local/tmp/pilotdroid/agent.jar
adb shell 'CLASSPATH=/data/local/tmp/pilotdroid/agent.jar setsid \
    app_process /data/local/tmp/pilotdroid pilotdroid.Agent serve'

# host 侧连通（占位行协议）
adb forward tcp:7900 localabstract:pilotdroid
printf 'HELLO\nTAP 700 1600\n' | nc localhost 7900

# 一次性注入自检（不经 socket）
adb shell 'CLASSPATH=/data/local/tmp/pilotdroid/agent.jar \
    app_process /data/local/tmp/pilotdroid pilotdroid.Agent inject 700 1600'
```

注意：`LocalServerSocket` 在 `app_process`（非 Android runtime app 上下文）下能否创建 unix socket 待 spike-2 一并验证；若不可行，回退为 `ServerSocket` 绑 `127.0.0.1` + `adb forward tcp:`。
