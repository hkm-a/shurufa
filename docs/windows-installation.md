# Windows 安装与卸载

## 安装

下载发行包中的 `Shurufa-Setup.exe`，右键选择“以管理员身份运行”。安装器会先停止旧
守护进程、备份旧部署，在新目录中预构建词典并注册 TSF；词典构建或注册失败时会恢复
旧版本。若上次安装中断而留下 `C:\ProgramData\shurufa.previous`，且正式安装目录已不存在，
安装器会先自动恢复该备份再继续升级；若两个目录同时存在，会要求确认是否删除旧备份。
升级时会重启 Windows 输入服务以释放旧 TSF DLL；新版本安装完成后会立即清理备份，若
系统仍持有文件则登记为下次重启后清理。
安装完成后可从“设置 → 应用 → 已安装的应用”卸载。

从当前源码构建单文件安装包：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1
```

产物位于 `dist\Shurufa-Setup.exe`。构建机需要已安装 NSIS；已经完成 Release 构建时，
可传入 `-SkipBuild`。为避免改变当前系统的输入法注册状态，构建命令只生成安装包，不会
执行安装。

也可以使用下面的管理员 PowerShell 7 脚本直接部署：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

安装脚本会构建 Windows TSF、独立算法服务和桌面守护进程，将 DLL、可执行文件、
librime 与方案暂存后部署至 `%ProgramData%\shurufa`，在切换前构建词典、授予
AppContainer 读取权限、注册 TSF，并启动 `shurufa-host supervise`。安装同时注册当前用户
的登录启动项，后续登录会自动恢复后台服务。算法服务由该 supervisor 使用同目录
`shurufa-algo.exe` 看护，因此不会发生多宿主进程争抢 Rime 用户词库锁的问题。

默认安装 Release 构建；开发调试使用：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\install.ps1 -Configuration Debug
```

若已完成构建，使用 `-SkipBuild`；若暂不在当前会话启动剪贴板与算法守护，使用
`-NoStartHost`（后续登录仍会自动启动）。脚本先将旧部署目录改名保留，若目录切换、ACL、
TSF 注册或登录启动配置失败，
会恢复旧目录并重新注册旧 DLL。系统或应用占用旧 DLL 导致目录无法切换时，请注销
后重试。

安装器与 PowerShell 部署脚本都会在 TSF 注册后立即激活当前会话，并把
“Shurufa 拼音”设为当前用户的默认输入法，无需再到 Windows 设置中添加或手动切换。
若安装以另一个管理员账户完成，Windows 只能修改该管理员账户的默认输入法；请使用
实际使用 Windows 的账户完成提升安装。

## Shurufa 设置页

从开始菜单或桌面运行 `Shurufa.exe` 可打开原生控制中心。它可保存或
关闭自托管中继地址、启动后台服务、触发内置热门词库 `rime-ice` 更新，并打开
Windows 的输入法设置。中继设置会在后台服务下次启动时生效。

## 卸载

在管理员终端中执行：

```cmd
scripts\unregister-dev.cmd
```

脚本会先请求已部署的 supervisor 停止 worker 与算法服务，再反注册 TSF 并删除
`%ProgramData%\shurufa`。若 DLL 仍被系统输入进程占用，注销后再执行一次。
