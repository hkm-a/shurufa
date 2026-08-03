param(
    [switch]$SkipBuild,
    [switch]$KeepEmulator,
    [switch]$Visible
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$androidRoot = Join-Path $repoRoot "platforms\android"
$artifacts = Join-Path $repoRoot ".claude\background-sync-artifacts"
$targetDir = Join-Path $repoRoot ".claude\background-sync-target"
$hostConfig = Join-Path $artifacts "windows-sync"
$hostDb = Join-Path $artifacts "windows-clipboard.db"
$hostLog = Join-Path $artifacts "windows-host.log"
$avdName = "shurufa_attachment_api35"
$androidPort = 48632
$forwardPort = 48633
$windowsPort = 48634
$testListenerTitle = "shurufa-background-sync-$windowsPort"
$testWorkerMutex = "Global\shurufa-background-sync-worker-$windowsPort"

function Find-ExistingPath {
    param([string[]]$Candidates)
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    return $null
}

function Invoke-Checked {
    param(
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$FailureMessage
    )
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FailureMessage，退出码：$LASTEXITCODE"
    }
}

function Wait-Until {
    param(
        [scriptblock]$Condition,
        [int]$TimeoutSeconds,
        [string]$FailureMessage
    )
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        try {
            if (& $Condition) { return }
        } catch {
            # 文件、服务和端口在启动窗口内暂不存在属于预期状态。
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    throw $FailureMessage
}

function Get-TargetEmulator {
    param([string]$Adb)
    $devices = & $Adb devices | Select-String "^(emulator-\d+)\s+device$"
    foreach ($device in $devices) {
        $serial = $device.Matches[0].Groups[1].Value
        $name = (& $Adb -s $serial emu avd name 2>$null | Select-Object -First 1).Trim()
        if ($name -eq $avdName) { return $serial }
    }
    return $null
}

function Get-CertificateFingerprint {
    param([string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-PeerFile {
    param(
        [string]$Path,
        [string]$Name,
        [string]$Fingerprint,
        [string]$Address
    )
    $peer = [ordered]@{
        name = $Name
        fingerprint = $Fingerprint
        last_addr = $Address
    }
    $json = $peer | ConvertTo-Json -Depth 4
    Set-Content -LiteralPath $Path -Value "[$json]" -Encoding utf8
}

function Get-AndroidLogs {
    param([string]$Adb, [string]$Serial)
    return (& $Adb -s $Serial logcat -d -v time "shurufa-sync:I" "shurufa-sync-test:I" "AndroidRuntime:E" "*:S") -join "`n"
}

function Request-TestImage {
    param(
        [string]$HostExe,
        [string]$ListenerTitle,
        [int]$Width,
        [int]$Height
    )
    $oldTitle = $env:SHURUFA_TEST_LISTENER_TITLE
    try {
        $env:SHURUFA_TEST_LISTENER_TITLE = $ListenerTitle
        & $HostExe test-set-image $Width $Height 2>$null | Out-Null
        return $LASTEXITCODE -eq 0
    } finally {
        if ($null -eq $oldTitle) {
            Remove-Item Env:SHURUFA_TEST_LISTENER_TITLE -ErrorAction SilentlyContinue
        } else {
            $env:SHURUFA_TEST_LISTENER_TITLE = $oldTitle
        }
    }
}

function Get-TestImageInspection {
    param(
        [string]$HostExe,
        [string]$ListenerTitle
    )
    $oldTitle = $env:SHURUFA_TEST_LISTENER_TITLE
    try {
        $env:SHURUFA_TEST_LISTENER_TITLE = $ListenerTitle
        $inspection = & $HostExe test-inspect-image 2>$null
        if ($LASTEXITCODE -eq 0) { return $inspection }
        return $null
    } finally {
        if ($null -eq $oldTitle) {
            Remove-Item Env:SHURUFA_TEST_LISTENER_TITLE -ErrorAction SilentlyContinue
        } else {
            $env:SHURUFA_TEST_LISTENER_TITLE = $oldTitle
        }
    }
}

$claudeCache = Join-Path $env:LOCALAPPDATA "Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local"
$androidSdk = Find-ExistingPath @(
    $env:ANDROID_HOME,
    $env:ANDROID_SDK_ROOT,
    (Join-Path $claudeCache "Android\Sdk"),
    (Join-Path $env:LOCALAPPDATA "Android\Sdk")
)
if (-not $androidSdk) { throw "未找到 Android SDK" }

$gradle = Find-ExistingPath @(
    (Join-Path $claudeCache "gradle-dist\gradle-8.10.2\bin\gradle.bat"),
    (Join-Path $env:LOCALAPPDATA "gradle-dist\gradle-8.10.2\bin\gradle.bat")
)
if (-not $gradle) { throw "未找到 Gradle 8.10.2" }

$adb = Join-Path $androidSdk "platform-tools\adb.exe"
$emulator = Join-Path $androidSdk "emulator\emulator.exe"
$hostExe = Join-Path $targetDir "debug\shurufa-host.exe"
$imeApk = Join-Path $androidRoot "app\build\outputs\apk\debug\app-debug.apk"
$testApk = Join-Path $androidRoot "attachment-test\build\outputs\apk\debug\attachment-test-debug.apk"

New-Item -ItemType Directory -Force -Path $artifacts, $hostConfig | Out-Null
Remove-Item -LiteralPath $hostLog, $hostDb -Force -ErrorAction SilentlyContinue

if (-not $SkipBuild) {
    $oldTarget = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = $targetDir
        Invoke-Checked "cargo" @("build", "-p", "shurufa-host") "构建隔离 Windows host 失败"
    } finally {
        $env:CARGO_TARGET_DIR = $oldTarget
    }
    Invoke-Checked $gradle @(
        "-p", $androidRoot,
        "lintDebug", "testDebugUnitTest", ":app:assembleDebug", ":attachment-test:assembleDebug",
        "--console=plain"
    ) "Android 构建或测试失败"
}

$startedEmulator = $false
$serial = Get-TargetEmulator $adb
if (-not $serial) {
    $serial = "emulator-5580"
    $arguments = @(
        "-avd", $avdName, "-port", "5580", "-no-audio", "-no-snapshot",
        "-no-boot-anim", "-gpu", "swiftshader_indirect"
    )
    if (-not $Visible) { $arguments += "-no-window" }
    Start-Process -FilePath $emulator -ArgumentList $arguments `
        -RedirectStandardOutput (Join-Path $artifacts "emulator.log") `
        -RedirectStandardError (Join-Path $artifacts "emulator-error.log") `
        -WindowStyle Hidden | Out-Null
    $startedEmulator = $true
}

$hostProcess = $null
$dailyHostProcess = $null
$dailyHostExe = Join-Path $repoRoot "target\debug\shurufa-host.exe"
$dailyHostWasRunning = $false
$forwardCreated = $false
$primaryError = $null
try {
    Invoke-Checked $adb @("-s", $serial, "wait-for-device") "等待模拟器连接失败"
    Wait-Until {
        (& $adb -s $serial shell getprop sys.boot_completed 2>$null).Trim() -eq "1"
    } 300 "模拟器五分钟内未完成开机"

    # 真实日常守护进程也监听 Windows 系统剪贴板。仅暂停当前仓库默认端口
    # 的 Shurufa 实例，防止测试图片被广播到真实配对设备；结束时原样恢复。
    $dailyListener = Get-NetTCPConnection -State Listen -LocalPort $androidPort -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($dailyListener) {
        $candidate = Get-Process -Id $dailyListener.OwningProcess -ErrorAction Stop
        if ($candidate.ProcessName -ne "shurufa-host" -or -not (Test-Path -LiteralPath $dailyHostExe)) {
            throw "默认同步端口被未知进程占用，拒绝继续：PID $($dailyListener.OwningProcess)"
        }
        $dailyHostProcess = $candidate
        $dailyHostWasRunning = $true
        Stop-Process -Id $candidate.Id -Force
        Wait-Until { -not (Get-Process -Id $candidate.Id -ErrorAction SilentlyContinue) } 10 "日常 Shurufa host 未停止"
    }

    Invoke-Checked $adb @("-s", $serial, "install", "-r", $imeApk) "安装输入法 APK 失败"
    Invoke-Checked $adb @("-s", $serial, "install", "-r", $testApk) "安装测试 APK 失败"
    & $adb -s $serial shell pm clear com.shurufa.ime | Out-Null
    & $adb -s $serial shell pm clear com.shurufa.attachmenttest | Out-Null
    Invoke-Checked $adb @("-s", $serial, "shell", "ime", "enable", "com.shurufa.ime/.ShurufaImeService") "启用输入法失败"
    Invoke-Checked $adb @("-s", $serial, "shell", "ime", "set", "com.shurufa.ime/.ShurufaImeService") "设置默认输入法失败"
    Wait-Until {
        ((& $adb -s $serial shell dumpsys input_method) -join "`n") -match "com.shurufa.ime/.ShurufaImeService"
    } 15 "系统未确认 Shurufa 为默认输入法"

    # 先创建两端隔离身份，再互相导入配对表。Android 启动页属于用户可见入口，
    # 满足 Android 12+ 前台服务启动限制；随后立即回到桌面并保持键盘关闭。
    Invoke-Checked $adb @("-s", $serial, "shell", "am", "start", "-n", "com.shurufa.ime/.PairActivity") "启动配对页失败"
    Wait-Until {
        (& $adb -s $serial shell run-as com.shurufa.ime ls files/sync/identity.cert.der 2>$null) -match "identity.cert.der"
    } 30 "Android 同步身份未生成"
    & $adb -s $serial exec-out run-as com.shurufa.ime cat files/sync/identity.cert.der > (Join-Path $artifacts "android.cert.der")

    $hostEnv = @{
        SHURUFA_SYNC_DIR = $hostConfig
        SHURUFA_SYNC_PORT = "$windowsPort"
        SHURUFA_DB_PATH = $hostDb
        SHURUFA_LOG_PATH = $hostLog
        SHURUFA_TEST_LISTENER_TITLE = $testListenerTitle
        SHURUFA_TEST_WORKER_MUTEX = $testWorkerMutex
    }
    $oldSyncDir = $env:SHURUFA_SYNC_DIR
    try {
        $env:SHURUFA_SYNC_DIR = $hostConfig
        Invoke-Checked $hostExe @("devices") "生成 Windows 测试身份失败"
    } finally {
        $env:SHURUFA_SYNC_DIR = $oldSyncDir
    }

    $androidFingerprint = Get-CertificateFingerprint (Join-Path $artifacts "android.cert.der")
    $windowsFingerprint = Get-CertificateFingerprint (Join-Path $hostConfig "identity.cert.der")
    Write-PeerFile (Join-Path $hostConfig "peers.json") "Android 模拟器" $androidFingerprint "127.0.0.1:$forwardPort"
    Write-PeerFile (Join-Path $artifacts "android-peers.json") "Windows 测试机" $windowsFingerprint "10.0.2.2:$windowsPort"
    Invoke-Checked $adb @("-s", $serial, "push", (Join-Path $artifacts "android-peers.json"), "/data/local/tmp/shurufa-peers.json") "推送 Android 配对表失败"
    Invoke-Checked $adb @("-s", $serial, "shell", "run-as", "com.shurufa.ime", "cp", "/data/local/tmp/shurufa-peers.json", "files/sync/peers.json") "导入 Android 配对表失败"

    Invoke-Checked $adb @("-s", $serial, "forward", "tcp:$forwardPort", "tcp:$androidPort") "建立 Android 端口转发失败"
    $forwardCreated = $true
    & $adb -s $serial logcat -c
    Invoke-Checked $adb @(
        "-s", $serial, "shell", "am", "broadcast",
        "-a", "com.shurufa.ime.DEBUG_START_SYNC",
        "-n", "com.shurufa.ime/.DebugSyncControlReceiver"
    ) "请求启动后台同步服务失败"
    Wait-Until {
        ((& $adb -s $serial shell dumpsys activity services com.shurufa.ime) -join "`n") -match "ClipboardSyncService"
    } 15 "Android 后台同步服务未启动"
    & $adb -s $serial shell input keyevent 3 | Out-Null

    $oldHostEnvironment = @{
        SHURUFA_SYNC_DIR = $env:SHURUFA_SYNC_DIR
        SHURUFA_SYNC_PORT = $env:SHURUFA_SYNC_PORT
        SHURUFA_DB_PATH = $env:SHURUFA_DB_PATH
        SHURUFA_LOG_PATH = $env:SHURUFA_LOG_PATH
    }
    try {
        foreach ($entry in $hostEnv.GetEnumerator()) {
            Set-Item -Path "Env:$($entry.Key)" -Value $entry.Value
        }
        $hostProcess = Start-Process -FilePath $hostExe -ArgumentList @("run") -PassThru -WindowStyle Hidden
    } finally {
        foreach ($entry in $oldHostEnvironment.GetEnumerator()) {
            if ($null -eq $entry.Value) {
                Remove-Item -Path "Env:$($entry.Key)" -ErrorAction SilentlyContinue
            } else {
                Set-Item -Path "Env:$($entry.Key)" -Value $entry.Value
            }
        }
    }
    Wait-Until {
        (Test-Path $hostLog) -and ((Get-Content $hostLog -Raw) -match "同步服务就绪")
    } 30 "Windows 测试同步服务未就绪"
    Wait-Until {
        $logs = Get-AndroidLogs $adb $serial
        $hostText = if (Test-Path $hostLog) { Get-Content $hostLog -Raw } else { "" }
        # 只接受真实连接事件，不能把“请求启动后台同步服务”当作握手完成。
        $logs -match "已连接" -or $hostText -match "同步：已连接"
    } 30 "Windows 与 Android 三十秒内未建立同步连接"

    $imeState = (& $adb -s $serial shell dumpsys input_method) -join "`n"
    if ($imeState -match "mInputShown=true" -or $imeState -match "mWindowVisible=true") {
        throw "测试前输入法窗口仍可见，无法证明后台同步"
    }

    # Windows -> Android：真实 CF_DIB 经同步服务到 Android image/* URI 剪贴板。
    Wait-Until {
        Request-TestImage $hostExe $testListenerTitle 41 29
    } 15 "Windows 常驻监听窗口未接受测试图片请求"
    Wait-Until {
        $logs = Get-AndroidLogs $adb $serial
        $logs -match "已接收 .* 的图片并写入系统剪贴板"
    } 30 "Android 未在后台收到 Windows 图片"
    & $adb -s $serial logcat -c
    Invoke-Checked $adb @(
        "-s", $serial, "shell", "am", "start", "-W",
        "-a", "com.shurufa.attachmenttest.INSPECT_CLIPBOARD",
        "-n", "com.shurufa.attachmenttest/.MainActivity"
    ) "启动 Android 剪贴板检查失败"
    Wait-Until {
        (Get-AndroidLogs $adb $serial) -match "检查剪贴板成功 .*解析=image/png.*宽=41 高=29"
    } 15 "Windows 图片到 Android 后不是 41x29 的 image/png"
    $clipboardInspection = Get-AndroidLogs $adb $serial
    if ($clipboardInspection -notmatch "检查剪贴板成功 .*URI=content://media/external/images/media/") {
        throw "Windows 图片未落入 Android MediaStore 图片 URI：`n$clipboardInspection"
    }
    $clipboardInspection | Set-Content -LiteralPath (Join-Path $artifacts "windows-to-android.log") -Encoding utf8
    Write-Host "通过：Windows 图片在键盘关闭时同步为 Android 图片剪贴板"

    # Android -> Windows：外部应用发布真实 FileProvider 图片 URI，输入法服务后台捕获。
    & $adb -s $serial logcat -c
    Invoke-Checked $adb @(
        "-s", $serial, "shell", "am", "start", "-W",
        "-a", "com.shurufa.attachmenttest.PUBLISH_IMAGE",
        "-n", "com.shurufa.attachmenttest/.MainActivity"
    ) "发布 Android 图片剪贴板失败"
    Wait-Until {
        $inspection = Get-TestImageInspection $hostExe $testListenerTitle
        $inspection -match "图片=37x23"
    } 30 "Android 图片未同步为 Windows 37x23 位图剪贴板"
    $windowsInspection = Get-TestImageInspection $hostExe $testListenerTitle
    if ($windowsInspection -notmatch "图片=37x23") {
        throw "Windows 剪贴板图片检查失败：$windowsInspection"
    }
    Write-Host "通过：Android 图片在键盘关闭时同步为 Windows 图片剪贴板"

    $finalImeState = (& $adb -s $serial shell dumpsys input_method) -join "`n"
    if ($finalImeState -match "mInputShown=true" -or $finalImeState -match "mWindowVisible=true") {
        throw "测试结束时输入法窗口被意外打开"
    }

    Get-AndroidLogs $adb $serial | Set-Content -LiteralPath (Join-Path $artifacts "android-sync.log") -Encoding utf8
    $windowsInspection | Set-Content -LiteralPath (Join-Path $artifacts "windows-clipboard.txt") -Encoding utf8
    Write-Host "后台双向图片剪贴板同步验收全部通过。"
    Write-Host "产物目录：$artifacts"
} catch {
    $primaryError = $_
    try { Get-AndroidLogs $adb $serial | Set-Content -LiteralPath (Join-Path $artifacts "android-failure.log") -Encoding utf8 } catch {}
    try { (& $adb -s $serial shell dumpsys activity services com.shurufa.ime) | Set-Content -LiteralPath (Join-Path $artifacts "android-services.txt") -Encoding utf8 } catch {}
} finally {
    try {
        if ($hostProcess -and -not $hostProcess.HasExited) {
            Stop-Process -Id $hostProcess.Id -Force
            Wait-Process -Id $hostProcess.Id -Timeout 10 -ErrorAction SilentlyContinue
        }
    } catch {}
    try {
        if ($forwardCreated) { & $adb -s $serial forward --remove "tcp:$forwardPort" 2>$null | Out-Null }
    } catch {}
    try {
        if ($dailyHostWasRunning) {
            Wait-Until {
                -not (Get-NetTCPConnection -State Listen -LocalPort $windowsPort -ErrorAction SilentlyContinue)
            } 10 "隔离 Windows host 未释放测试端口"
            Start-Process -FilePath $dailyHostExe -ArgumentList @("run") -WindowStyle Hidden | Out-Null
            Wait-Until {
                Get-NetTCPConnection -State Listen -LocalPort $androidPort -ErrorAction SilentlyContinue
            } 15 "日常 Windows host 未恢复默认端口"
        }
    } catch {}
    try {
        if ($startedEmulator -and -not $KeepEmulator) {
            & $adb -s $serial emu kill | Out-Null
            Wait-Until {
                ((& $adb devices) -join "`n") -notmatch [Regex]::Escape("$serial`tdevice")
            } 30 "测试模拟器三十秒内未退出"
        }
    } catch {}
}

if ($primaryError) { throw $primaryError }
