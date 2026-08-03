# Windows 安装与卸载

## 安装

下载发行包中的 `Shurufa-Setup.exe`，右键选择“以管理员身份运行”。升级时安装器会先停止旧
守护进程、释放当前 TSF 注册和输入服务句柄，再原位覆盖安装目录中的程序文件、预构建词典并
重新注册 TSF。安装过程不创建、重命名或删除 `shurufa.previous` 等备用目录；用户数据和现有
安装目录不会因升级失败被递归清理。若系统仍占用输入法 DLL，关闭占用程序后重新运行安装器即可。
安装完成后可从“设置 → 应用 → 已安装的应用”卸载。
后台宿主由安装器以隐藏窗口启动；任务管理器中看到 supervisor 和 worker 两个 `shurufa-host.exe`
属于同一套监管进程，不会显示前台控制台窗口。

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
`-NoStartHost`（后续登录仍会自动启动）。脚本与发行安装器一样执行原位更新：停止旧宿主、
释放输入服务、覆盖程序文件并重新注册 TSF，不再创建或清理备用安装目录。系统或应用占用旧
DLL 时，关闭占用程序后重试即可。

安装器与 PowerShell 部署脚本都会在 TSF 注册后立即激活当前会话，并把
“Shurufa 拼音”设为当前用户的默认输入法，无需再到 Windows 设置中添加或手动切换。
若安装以另一个管理员账户完成，Windows 只能修改该管理员账户的默认输入法；请使用
实际使用 Windows 的账户完成提升安装。

## Shurufa 设置页

从开始菜单或桌面运行 `Shurufa.exe` 可打开 Tauri 控制中心。控制中心提供工作台、输入、词库、
跨设备和偏好五个页面，可保存或关闭自托管中继地址、启动后台服务、触发内置热门
词库 `rime-ice` 更新，并打开 Windows 的输入法设置。中继设置会在后台服务下次启动时生效。

## 卸载

在管理员终端中执行：

```cmd
scripts\unregister-dev.cmd
```

脚本会先请求已部署的 supervisor 停止 worker 与算法服务，再反注册 TSF 并删除
`%ProgramData%\shurufa`。若 DLL 仍被系统输入进程占用，注销后再执行一次。
