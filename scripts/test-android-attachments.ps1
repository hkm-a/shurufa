param(
    [switch]$Logs,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$androidRoot = Join-Path $repoRoot "platforms\android"

function Find-ExistingPath {
    param([string[]]$Candidates)
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    return $null
}

$claudeCache = Join-Path $env:LOCALAPPDATA "Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local"
$androidSdk = Find-ExistingPath @(
    $env:ANDROID_HOME,
    $env:ANDROID_SDK_ROOT,
    (Join-Path $claudeCache "Android\Sdk"),
    (Join-Path $env:LOCALAPPDATA "Android\Sdk")
)
if (-not $androidSdk) {
    throw "未找到 Android SDK。请设置 ANDROID_HOME，或确认 Claude Code 缓存中的 SDK 仍存在。"
}

$gradle = Find-ExistingPath @(
    (Join-Path $claudeCache "gradle-dist\gradle-8.10.2\bin\gradle.bat"),
    (Join-Path $env:LOCALAPPDATA "gradle-dist\gradle-8.10.2\bin\gradle.bat")
)
if (-not $gradle) {
    throw "未找到 Gradle 8.10.2。"
}

$adb = Join-Path $androidSdk "platform-tools\adb.exe"
if (-not (Test-Path -LiteralPath $adb)) {
    throw "未找到 ADB：$adb"
}

$env:ANDROID_HOME = $androidSdk
$env:ANDROID_SDK_ROOT = $androidSdk

if ($Logs) {
    & $adb logcat -c
    Write-Host "正在显示输入法与附件测试器日志，按 Ctrl+C 停止。"
    & $adb logcat "shurufa:D" "shurufa-attachment-test:D" "AndroidRuntime:E" "InputConnectionCompat:V" "*:S"
    exit $LASTEXITCODE
}

$devices = @(& $adb devices | Select-String "\tdevice$")
if ($devices.Count -eq 0) {
    throw "未检测到已授权的 Android 设备。连接手机并允许 USB 调试后重新运行。"
}
if ($devices.Count -gt 1) {
    throw "检测到多个 Android 设备。请暂时只连接一台设备。"
}

if (-not $SkipBuild) {
    & $gradle -p $androidRoot :app:assembleDebug :attachment-test:assembleDebug --console=plain
    if ($LASTEXITCODE -ne 0) {
        throw "Android APK 构建失败。"
    }
}

$imeApk = Join-Path $androidRoot "app\build\outputs\apk\debug\app-debug.apk"
$testApk = Join-Path $androidRoot "attachment-test\build\outputs\apk\debug\attachment-test-debug.apk"
foreach ($apk in @($imeApk, $testApk)) {
    if (-not (Test-Path -LiteralPath $apk)) {
        throw "未找到 APK：$apk"
    }
    & $adb install -r $apk
    if ($LASTEXITCODE -ne 0) {
        throw "安装失败：$apk"
    }
}

& $adb shell am force-stop com.shurufa.attachmenttest
& $adb shell am start -n com.shurufa.attachmenttest/.MainActivity
if ($LASTEXITCODE -ne 0) {
    throw "测试器启动失败。"
}

Write-Host ""
Write-Host "测试器已打开。"
Write-Host "1. 点击“标准富内容输入框”，再点输入法中的同步图片或文件。"
Write-Host "2. 点击“剪贴板附件回退输入框”，重复同一操作。"
Write-Host "3. 测试器显示 URI、MIME 和字节数即代表目标进程实际读取成功。"
Write-Host "4. 查看日志：powershell -ExecutionPolicy Bypass -File scripts\test-android-attachments.ps1 -Logs"
