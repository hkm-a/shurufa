# M3 验收：Android 拼音键盘

产物：`platforms/android/app/build/outputs/apk/debug/app-debug.apk`
（arm64-v8a，minSdk 23 即 Android 6.0+，约 7.5MB）

重新构建：`scripts/build-android.cmd`（Rust 交叉编译 → 拷贝 so → Gradle）。

## 1. 安装

手机开启「开发者选项 → USB 调试」后连接电脑：

```bash
%LOCALAPPDATA%/Android/Sdk/platform-tools/adb.exe install -r platforms/android/app/build/outputs/apk/debug/app-debug.apk
```

或把 APK 传到手机（微信/网盘/数据线均可）直接点击安装
（debug 签名，系统会提示"未知来源"，允许即可）。

## 2. 启用

1. 系统设置 → 系统 → 语言和输入法 → 屏幕键盘（各厂商叫法略异）
2. 启用「Shurufa 输入法」，系统会弹安全提示（输入法可读取键入内容），确认
3. 在任意输入框呼出键盘，用键盘切换按钮（或通知栏）切到「Shurufa 拼音」

## 3. 打字验收

1. **首次使用**：引擎后台部署词典（数秒到数十秒），期间按键为英文直通；
   部署完成后自动进入拼音模式（无需重启键盘）
2. 点按 `nihao`：候选栏出现「你好」等候选，输入框内显示拼音组合串
3. 点候选或按「空格」上屏「你好」
4. 「回车」在组合中把拼音字母原样上屏（与桌面端一致），无组合时为换行
5. 「⌫」组合中删拼音字母，无组合时删除已有文本
6. 「，」「。」组合中先确认首选再上屏标点
7. 切换到别的输入框，残留组合应被清空

## 已知边界（M3 范围外）

- 无大写/符号面板/数字排：MVP 只有字母三排 + 功能行
- 无按键气泡、滑动、长按；无深色模式适配
- 候选栏不分页（横向滚动查看更多）
- 仅 arm64-v8a（2016 年后的主流机型均可）
- 与桌面端的词库同步、剪贴板同步属 M4
