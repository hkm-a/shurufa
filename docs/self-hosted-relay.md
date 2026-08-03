# 自托管同步中继

`shurufa-relay` 用于已配对设备位于不同网络、且局域网直连不可用时的剪贴板同步回退。它只按设备证书指纹配对两条 TCP 连接，随后透明转发字节；设备之间仍使用既有的双向 TLS 和证书指纹校验。

中继不保存剪贴板内容、设备私钥或配对表。断开连接后，中继不会保留任何转发数据。

## 构建与运行

在仓库根目录构建：

```powershell
cargo build -p shurufa-relay --release
```

在一台具有公网可达地址的主机上运行，默认端口为 `48633`：

```powershell
.\target\release\shurufa-relay.exe
```

也可传入明确的监听地址：

```powershell
.\target\release\shurufa-relay.exe 0.0.0.0:48633
```

部署时应让两台设备都能访问该 TCP 地址，例如 `relay.example.com:48633`。如果运行在容器或云主机中，需要将该端口映射到宿主机并允许入站 TCP 流量。

## 配置客户端

首次配对必须让两台设备能互相直连：在同一局域网内使用自动发现，或在配对页/命令行指定对方的 `IP:48632`。配对完成后，中继才可按双方已保存的证书指纹建立跨网段通道。

Windows：

```powershell
shurufa-host relay relay.example.com:48633
```

命令会保存 `%APPDATA%\shurufa\sync\relay.addr`。重启 `shurufa-host` 后生效。关闭中继：

```powershell
shurufa-host relay off
```

Android：打开“剪贴板同步”页，在“自托管中继（跨网段可选）”填入同一地址并保存，然后完全重启输入法。输入空值或 `off` 可关闭。

客户端总是优先使用局域网直连；仅当没有可用直连地址或直连失败时才使用中继。中继通道不会改写设备记录中的最近直连地址。

## 本地验证

```powershell
cargo test -p sync-core
cargo build -p shurufa-relay
```

其中 `已配对设备可仅经中继双向同步文本` 会以临时身份、临时配对表和本地中继验证双向 TLS 同步，且确认不会写入直连地址。
