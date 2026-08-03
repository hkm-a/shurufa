# 自托管云词库

Shurufa 的云词库不是在线候选服务，也不会上传输入内容。它是一个可自托管的 Rime YAML 词库发布机制：客户端从 HTTPS 下载版本清单及词典文件，逐文件校验字节数和 SHA-256 后才替换本地词典，并在下一次启动输入法时重新部署 Rime。

## 默认热门来源

默认值 `rime-ice` 对应 [iDvel/rime-ice](https://github.com/iDvel/rime-ice) 的 `2026.06.30` 稳定词典快照。该项目约有 1.86 万 Star，持续发布 `cn_dicts.zip`、`full.zip` 等词典包；Shurufa 已内置其四个当前方案直接依赖的中文词典文件的固定版本、原始 HTTPS 地址、精确大小和 SHA-256，因此可直接更新而无需自建清单。

默认清单优先使用经本地完整校验的 jsDelivr 固定版本地址，并保留 GitHub Raw 为回退。镜像仅改善可用性，任何来源的文件都必须同时通过精确大小和 SHA-256 校验才会写入词典暂存区。

[gaboolic/rime-frost](https://github.com/gaboolic/rime-frost) 约有 3479 Star，适合作为高质量的**整套输入方案迁移**选项，但它包含独立方案、配置和可选语法模型，不能作为 `rime_ice` 的普通词典覆盖包。较旧的 [rime-aca/dictionaries](https://github.com/rime-aca/dictionaries) 仍可作扩展词条参考，但维护活跃度较低，不作为默认更新源。

## 清单格式

将 `manifest.json` 与 YAML 文件部署到任意 HTTPS 静态站点。版本 1 的完整清单如下：

```json
{
  "version": 1,
  "revision": "2026-08-03-行业词",
  "files": [
    {
      "path": "cn_dicts/custom.yaml",
      "url": "https://dict.example.com/cn_dicts/custom.yaml",
      "sha256": "64 位十六进制 SHA-256",
      "size": 词典的 UTF-8 字节数
    }
  ]
}
```

每次发布应列出该版本全部需要覆盖的 YAML 文件。路径只能是相对路径，不能含 `..`、空段或反斜杠；单文件上限 32 MiB，单清单最多 32 个文件。清单和每个文件都必须使用 HTTPS。

发布前可在 PowerShell 生成校验字段：

```powershell
$file = "cn_dicts\custom.yaml"
$hash = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $file).Length
```

## Windows 更新

部署完成并确保 `rime_deployer.exe` 位于 `%ProgramData%\shurufa` 后执行：

```powershell
shurufa-host dict-update rime-ice
```

也可以使用自托管清单：

```powershell
shurufa-host dict-update https://dict.example.com/manifest.json
```

命令会先下载所有文件至暂存目录（网络瞬断最多自动重试三次），全部通过清单校验后逐文件原子替换 `%ProgramData%\shurufa\schemas` 内的 YAML，随后调用 `rime_deployer.exe --build`。部署器缺失会在替换前报错；若编译失败，则自动恢复更新前的所有词典文件。命令成功后重启输入法；失败时修复清单或部署权限后重试。

开发环境可设置 `SHURUFA_SCHEMAS` 指向可写的方案目录。回退到上一词库版本时，重新运行旧版本的清单即可。

## Android 更新

打开应用的“剪贴板同步”页面，在“云词库更新（可选）”保留 `rime-ice` 并选择“更新云词库”，即可更新默认热门词典；也可输入自托管 HTTPS 清单地址。下载、大小校验和 SHA-256 校验在后台完成；成功后必须完全重启输入法。

Android 将远端词典放在应用私有目录。引擎下次初始化时才把通过校验的覆盖包合并到解包方案并删除旧的 Rime 编译产物，因此运行中的输入会话不会遇到半更新词典。

## 本地验证

```powershell
cargo test -p shurufa-host dict_update -- --nocapture
cargo test -p shurufa-host dict_update -- --ignored --nocapture
& "$env:TEMP\shurufa-gradle-8.10.2\gradle-8.10.2\bin\gradle.bat" -p platforms\android :app:testDebugUnitTest --console=plain
```

第一条 Rust 单测与 Android 单测分别覆盖 HTTPS 限制、路径穿越拒绝、清单解析、精确长度校验、SHA-256 校验和 Windows 回滚；第二条 Rust 命令会实际下载内置 `rime-ice` 的固定版本并进行全量校验，因此需要网络。
