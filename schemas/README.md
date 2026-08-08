# 输入方案与词库

本目录是 Windows 与 Android 共用的 Rime 方案唯一来源，构建期分别被打包进
NSIS 安装器（`installer\shurufa.nsi`）与 Android assets（`platforms\android\app\build.gradle.kts` 的 `syncSchemas` 任务）。

| 条目 | 说明 |
| --- | --- |
| `rime_ice.schema.yaml` + `rime_ice.dict.yaml` + `cn_dicts\*.dict.yaml` | 默认雾凇拼音方案与词库，版本见 `rime-ice-*.json` manifest |
| `luna_pinyin.*` / `pinyin_simp.*` / `stroke.*` | 备用方案（Windows 侧可选启用） |
| `default.yaml` / `pinyin.yaml` / `key_bindings.yaml` / `punctuation.yaml` / `symbols.yaml` | 全局与方案通用配置 |
| `essay.txt` | 语言模型语料（八股文） |
| `shurufa-skin.json` | 跨端皮肤单一入口，Android 与 Windows 控制中心读取 |

## 词库版本与更新

`rime-ice-2026.06.30.json` 是当前锁定的 rime-ice manifest：列出每个 `cn_dicts` 文件的
jsDelivr 主 URL、GitHub Raw 回退 URL、SHA-256 与字节数。客户端按 manifest 校验后增量更新。
更新词库版本 = 替换 manifest + 对应 `cn_dicts` 文件，详见 [云词库](../docs/云词库.md)。

部署期由 `rime_deployer.exe --build` 在本目录生成 `build/`（已被 .gitignore 排除）；
用户数据（`user.yaml`、自造词）不写入本目录，Windows 落在 `%APPDATA%\Rime`，Android 落在应用私有目录。
