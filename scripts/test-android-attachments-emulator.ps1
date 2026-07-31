param(
    [switch]$SkipBuild,
    [switch]$KeepEmulator,
    [switch]$Visible
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$androidRoot = Join-Path $repoRoot "platforms\android"
$artifacts = Join-Path $repoRoot ".claude\android-emulator-artifacts"
$avdName = "shurufa_attachment_api35"
$imagePackage = "system-images;android-35;google_apis;x86_64"
$prebuiltCommit = "86ce2c95d42f1132746fbf60c278193aa1f4b758"
$referenceWidth = 1080
$referenceHeight = 2400

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
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $FilePath @Arguments 2>&1 | ForEach-Object { "$_" }
    } finally {
        $ErrorActionPreference = $previous
    }
    if ($LASTEXITCODE -ne 0) {
        throw "$FailureMessage，退出码：$LASTEXITCODE"
    }
}

function Get-TargetEmulator {
    param([string]$Adb)
    $devices = & $Adb devices | Select-String "^(emulator-\d+)\s+device$"
    foreach ($device in $devices) {
        $serial = $device.Matches[0].Groups[1].Value
        $name = (& $Adb -s $serial emu avd name 2>$null | Select-Object -First 1).Trim()
        if ($name -eq $avdName) {
            return $serial
        }
    }
    return $null
}

function Wait-ForBoot {
    param(
        [string]$Adb,
        [string]$Serial
    )
    $deadline = (Get-Date).AddMinutes(5)
    do {
        Start-Sleep -Seconds 2
        $state = try {
            (& $Adb -s $Serial get-state 2>$null)
        } catch {
            $null
        }
        if ($state -eq "device") {
            $boot = (& $Adb -s $Serial shell getprop sys.boot_completed 2>$null).Trim()
            if ($boot -eq "1") {
                return
            }
        }
    } while ((Get-Date) -lt $deadline)
    throw "模拟器五分钟内未完成开机：$Serial"
}

function Wait-ForImeVisible {
    param(
        [string]$Adb,
        [string]$Serial
    )
    $deadline = (Get-Date).AddSeconds(60)
    do {
        Start-Sleep -Seconds 1
        $state = (& $Adb -s $Serial shell dumpsys input_method) -join "`n"
        if ($state -match "mInputShown=true" -and $state -match "mWindowVisible=true") {
            return
        }
    } while ((Get-Date) -lt $deadline)
    throw "输入法窗口六十秒内未显示"
}

function Get-ScreenSize {
    param(
        [string]$Adb,
        [string]$Serial
    )
    $line = (& $Adb -s $Serial shell wm size | Select-String "Physical size: (\d+)x(\d+)").Matches
    if ($line.Count -eq 0) {
        throw "无法读取模拟器屏幕尺寸"
    }
    return @(
        [int]$line[0].Groups[1].Value,
        [int]$line[0].Groups[2].Value
    )
}

function Invoke-Tap {
    param(
        [string]$Adb,
        [string]$Serial,
        [int]$Width,
        [int]$Height,
        [int]$ReferenceX,
        [int]$ReferenceY
    )
    $x = [Math]::Round($ReferenceX * $Width / $referenceWidth)
    $y = [Math]::Round($ReferenceY * $Height / $referenceHeight)
    Invoke-Checked $Adb @("-s", $Serial, "shell", "input", "tap", $x, $y) "点击设备坐标失败"
}

function Save-Screenshot {
    param(
        [string]$Adb,
        [string]$Serial,
        [string]$Name
    )
    $devicePath = "/sdcard/$Name.png"
    Invoke-Checked $Adb @("-s", $Serial, "shell", "screencap", "-p", $devicePath) "设备截图失败"
    Invoke-Checked $Adb @("-s", $Serial, "pull", $devicePath, (Join-Path $artifacts "$Name.png")) "拉取设备截图失败"
}

function Get-TestLogs {
    param(
        [string]$Adb,
        [string]$Serial
    )
    return (& $Adb -s $Serial logcat -d -v brief "shurufa-attachment-test:I" "*:S") -join "`n"
}

function Get-ImeLogs {
    param(
        [string]$Adb,
        [string]$Serial
    )
    return (& $Adb -s $Serial logcat -d -v brief "shurufa:I" "*:S") -join "`n"
}

function Wait-TestLogs {
    param(
        [string]$Adb,
        [string]$Serial,
        [string[]]$Expected
    )
    $deadline = (Get-Date).AddSeconds(12)
    do {
        Start-Sleep -Milliseconds 500
        $logs = Get-TestLogs $Adb $Serial
        $missing = @($Expected | Where-Object { $logs -notmatch [Regex]::Escape($_) })
        if ($missing.Count -eq 0 -and $logs -match "读取大小：[1-9][0-9]* 字节") {
            return $logs
        }
    } while ((Get-Date) -lt $deadline)
    throw "测试器日志缺少：$($missing -join '、')`n当前日志：`n$logs"
}

function Start-TestActivity {
    param(
        [string]$Adb,
        [string]$Serial
    )
    & $Adb -s $Serial shell am force-stop com.shurufa.attachmenttest | Out-Null
    Invoke-Checked $Adb @("-s", $Serial, "shell", "am", "start", "-n", "com.shurufa.attachmenttest/.MainActivity") "启动附件测试器失败"
    Start-Sleep -Seconds 2
}

function Test-Delivery {
    param(
        [string]$Name,
        [string]$Adb,
        [string]$Serial,
        [int]$Width,
        [int]$Height,
        [int]$InputY,
        [int]$EntryY,
        [string[]]$Expected
    )
    Start-TestActivity $Adb $Serial
    Invoke-Tap $Adb $Serial $Width $Height 540 $InputY
    Start-Sleep -Seconds 2
    Invoke-Tap $Adb $Serial $Width $Height 540 $InputY
    Wait-ForImeVisible $Adb $Serial
    Invoke-Tap $Adb $Serial $Width $Height 70 1650
    Start-Sleep -Seconds 1
    & $Adb -s $Serial logcat -c
    Invoke-Tap $Adb $Serial $Width $Height 300 $EntryY
    $logs = Wait-TestLogs $Adb $Serial $Expected
    $imeLogs = Get-ImeLogs $Adb $Serial
    if ($imeLogs -notmatch "附件富内容投递") {
        throw "输入法日志未记录附件投递决策：`n$imeLogs"
    }
    Save-Screenshot $Adb $Serial $Name
    Set-Content -LiteralPath (Join-Path $artifacts "$Name.log") -Value $logs -Encoding utf8
    Set-Content -LiteralPath (Join-Path $artifacts "$Name-ime.log") -Value $imeLogs -Encoding utf8
    Write-Host "通过：$Name"
}

function Test-UndeclaredDelivery {
    param(
        [string]$Name,
        [string]$Adb,
        [string]$Serial,
        [int]$Width,
        [int]$Height,
        [int]$InputY,
        [int]$EntryY,
        [string[]]$Expected
    )
    # 模拟抖音/B 站评论框：不声明 MIME 但实现 OnReceiveContentListener。
    # 输入法应无条件尝试 commitContent 并成功插入（微信输入法的通用做法）。
    Start-TestActivity $Adb $Serial
    Invoke-Tap $Adb $Serial $Width $Height 540 $InputY
    Start-Sleep -Seconds 2
    Invoke-Tap $Adb $Serial $Width $Height 540 $InputY
    Wait-ForImeVisible $Adb $Serial
    Invoke-Tap $Adb $Serial $Width $Height 70 1650
    Start-Sleep -Seconds 1
    & $Adb -s $Serial logcat -c
    Invoke-Tap $Adb $Serial $Width $Height 300 $EntryY
    $logs = Wait-TestLogs $Adb $Serial $Expected
    $imeLogs = Get-ImeLogs $Adb $Serial
    if ($imeLogs -notmatch "附件富内容投递") {
        throw "输入法日志未记录 commitContent 投递：`n$imeLogs"
    }
    Set-Content -LiteralPath (Join-Path $artifacts "$Name.log") -Value $logs -Encoding utf8
    Set-Content -LiteralPath (Join-Path $artifacts "$Name-ime.log") -Value $imeLogs -Encoding utf8
    Save-Screenshot $Adb $Serial $Name
    Write-Host "通过：$Name"
}

function Get-SyncTestLogs {
    param(
        [string]$Adb,
        [string]$Serial
    )
    return (& $Adb -s $Serial logcat -d -v brief "shurufa-sync-test:I" "*:S") -join "`n"
}

function Wait-ImeLogs {
    param(
        [string]$Adb,
        [string]$Serial,
        [string[]]$Expected
    )
    $deadline = (Get-Date).AddSeconds(12)
    do {
        Start-Sleep -Milliseconds 500
        $logs = Get-ImeLogs $Adb $Serial
        $missing = @($Expected | Where-Object { $logs -notmatch [Regex]::Escape($_) })
        if ($missing.Count -eq 0) {
            return $logs
        }
    } while ((Get-Date) -lt $deadline)
    throw "输入法日志缺少：$($missing -join '、')`n当前日志：`n$logs"
}

function Wait-SyncTestLogs {
    param(
        [string]$Adb,
        [string]$Serial,
        [string[]]$Expected
    )
    $deadline = (Get-Date).AddSeconds(12)
    do {
        Start-Sleep -Milliseconds 500
        $logs = Get-SyncTestLogs $Adb $Serial
        $missing = @($Expected | Where-Object { $logs -notmatch [Regex]::Escape($_) })
        if ($missing.Count -eq 0) {
            return $logs
        }
    } while ((Get-Date) -lt $deadline)
    throw "同步测试日志缺少：$($missing -join '、')`n当前日志：`n$logs"
}

function Test-ClipboardFallback {
    param(
        [string]$Name,
        [string]$Adb,
        [string]$Serial,
        [int]$Width,
        [int]$Height,
        [int]$InputY,
        [int]$EntryY
    )
    # 普通文本输入框（无附件能力、无分享入口）：点图后应回退为复制到剪贴板
    Start-TestActivity $Adb $Serial
    Invoke-Tap $Adb $Serial $Width $Height 540 $InputY
    Start-Sleep -Seconds 2
    Invoke-Tap $Adb $Serial $Width $Height 540 $InputY
    Wait-ForImeVisible $Adb $Serial
    Invoke-Tap $Adb $Serial $Width $Height 70 1650
    Start-Sleep -Seconds 1
    & $Adb -s $Serial logcat -c
    Invoke-Tap $Adb $Serial $Width $Height 300 $EntryY
    $imeLogs = Wait-ImeLogs $Adb $Serial @("图片已复制到剪贴板")
    Invoke-Checked $Adb @("-s", $Serial, "shell", "am", "start", "-W", "-a", "com.shurufa.attachmenttest.INSPECT_CLIPBOARD", "-n", "com.shurufa.attachmenttest/.MainActivity") "启动剪贴板检查失败"
    $syncLogs = Wait-SyncTestLogs $Adb $Serial @("检查剪贴板成功", "标签=Shurufa 同步·")
    if ($syncLogs -notmatch "字节=[1-9][0-9]*") {
        throw "剪贴板图片字节数为空：`n$syncLogs"
    }
    Save-Screenshot $Adb $Serial $Name
    Set-Content -LiteralPath (Join-Path $artifacts "$Name-ime.log") -Value $imeLogs -Encoding utf8
    Set-Content -LiteralPath (Join-Path $artifacts "$Name.log") -Value $syncLogs -Encoding utf8
    Write-Host "通过：$Name"
}

$claudeCache = Join-Path $env:LOCALAPPDATA "Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local"
$androidSdk = Find-ExistingPath @(
    $env:ANDROID_HOME,
    $env:ANDROID_SDK_ROOT,
    (Join-Path $claudeCache "Android\Sdk"),
    (Join-Path $env:LOCALAPPDATA "Android\Sdk")
)
if (-not $androidSdk) {
    throw "未找到 Android SDK"
}

$gradle = Find-ExistingPath @(
    (Join-Path $claudeCache "gradle-dist\gradle-8.10.2\bin\gradle.bat"),
    (Join-Path $env:LOCALAPPDATA "gradle-dist\gradle-8.10.2\bin\gradle.bat")
)
if (-not $gradle) {
    throw "未找到 Gradle 8.10.2"
}

$env:ANDROID_HOME = $androidSdk
$env:ANDROID_SDK_ROOT = $androidSdk
$sdkManager = Join-Path $androidSdk "cmdline-tools\latest\bin\sdkmanager.bat"
$avdManager = Join-Path $androidSdk "cmdline-tools\latest\bin\avdmanager.bat"
$adb = Join-Path $androidSdk "platform-tools\adb.exe"
$emulator = Join-Path $androidSdk "emulator\emulator.exe"
$systemImage = Join-Path $androidSdk "system-images\android-35\google_apis\x86_64"
$ndkBin = Join-Path $androidSdk "ndk\28.0.13004108\toolchains\llvm\prebuilt\windows-x86_64\bin"

New-Item -ItemType Directory -Force -Path $artifacts | Out-Null

if (-not (Test-Path -LiteralPath $emulator) -or -not (Test-Path -LiteralPath $systemImage)) {
    Invoke-Checked $sdkManager @("emulator", $imagePackage) "安装 Android Emulator 或系统镜像失败"
}

$librimeRoot = Join-Path $repoRoot "third_party\librime-android\x86_64"
$prebuiltFiles = [ordered]@{
    "librime/x86_64/include/rime_api.h" = "include/rime_api.h"
    "librime/x86_64/lib/librime.a" = "lib/librime.a"
    "glog/x86_64/lib/libglog.a" = "lib/libglog.a"
    "yaml-cpp/x86_64/lib/libyaml-cpp.a" = "lib/libyaml-cpp.a"
    "leveldb/x86_64/lib/libleveldb.a" = "lib/libleveldb.a"
    "marisa/x86_64/lib/libmarisa.a" = "lib/libmarisa.a"
    "opencc/x86_64/lib/libopencc.a" = "lib/libopencc.a"
    "boost/x86_64/lib/libboost_container.a" = "lib/libboost_container.a"
    "boost/x86_64/lib/libboost_iostreams.a" = "lib/libboost_iostreams.a"
    "boost/x86_64/lib/libboost_random.a" = "lib/libboost_random.a"
    "zstd/x86_64/lib/libzstd.a" = "lib/libzstd.a"
    "libiconv/x86_64/lib/libiconv.a" = "lib/libiconv.a"
}
foreach ($entry in $prebuiltFiles.GetEnumerator()) {
    $destination = Join-Path $librimeRoot $entry.Value
    if (-not (Test-Path -LiteralPath $destination)) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        $url = "https://raw.githubusercontent.com/fcitx5-android/prebuilt/$prebuiltCommit/$($entry.Key)"
        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $destination
    }
}

if ((rustup target list --installed) -notcontains "x86_64-linux-android") {
    Invoke-Checked "rustup" @("target", "add", "x86_64-linux-android") "安装 Rust Android x86_64 目标失败"
}

if (-not $SkipBuild) {
    $env:CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER = Join-Path $ndkBin "x86_64-linux-android23-clang.cmd"
    $env:CC_x86_64_linux_android = Join-Path $ndkBin "x86_64-linux-android23-clang.cmd"
    $env:CXX_x86_64_linux_android = Join-Path $ndkBin "x86_64-linux-android23-clang++.cmd"
    $env:AR_x86_64_linux_android = Join-Path $ndkBin "llvm-ar.exe"
    $env:CFLAGS_x86_64_linux_android = "--target=x86_64-linux-android23"

    Push-Location $repoRoot
    try {
        Invoke-Checked "cargo" @("build", "--release", "--target", "x86_64-linux-android", "-p", "shurufa-rimejni") "构建 x86_64 JNI 失败"
        $jniDir = Join-Path $androidRoot "app\src\main\jniLibs\x86_64"
        New-Item -ItemType Directory -Force -Path $jniDir | Out-Null
        Copy-Item -LiteralPath (Join-Path $repoRoot "target\x86_64-linux-android\release\libshurufa_rime.so") -Destination (Join-Path $jniDir "libshurufa_rime.so") -Force
        Invoke-Checked $gradle @("-p", $androidRoot, "lintDebug", "testDebugUnitTest", ":app:assembleDebug", ":attachment-test:assembleDebug", "--console=plain") "Android 构建或测试失败"
    } finally {
        Pop-Location
    }
}

$avdConfig = Join-Path $env:USERPROFILE ".android\avd\$avdName.avd\config.ini"
if (-not (Test-Path -LiteralPath $avdConfig)) {
    "no" | & $avdManager create avd --force --name $avdName --package $imagePackage --device "pixel_6"
    if ($LASTEXITCODE -ne 0) {
        throw "创建 Android 虚拟设备失败"
    }
}

$startedEmulator = $false
$serial = Get-TargetEmulator $adb
if (-not $serial) {
    $serial = "emulator-5580"
    $arguments = @("-avd", $avdName, "-port", "5580", "-no-audio", "-no-snapshot", "-no-boot-anim", "-gpu", "swiftshader_indirect")
    if (-not $Visible) {
        $arguments += "-no-window"
    }
    Start-Process -FilePath $emulator -ArgumentList $arguments -RedirectStandardOutput (Join-Path $artifacts "emulator.log") -RedirectStandardError (Join-Path $artifacts "emulator-error.log") -WindowStyle Hidden | Out-Null
    $startedEmulator = $true
}

try {
    Wait-ForBoot $adb $serial
    # userdebug 镜像从快照恢复后，shell 可能失去组件状态变更权限，pm enable/disable 需先切 root
    & $adb -s $serial root | Out-Null
    Start-Sleep -Seconds 3
    Wait-ForBoot $adb $serial
    & $adb -s $serial shell settings put global window_animation_scale 0 | Out-Null
    & $adb -s $serial shell settings put global transition_animation_scale 0 | Out-Null
    & $adb -s $serial shell settings put global animator_duration_scale 0 | Out-Null
    & $adb -s $serial shell svc power stayon true | Out-Null
    & $adb -s $serial shell input keyevent 82 | Out-Null
    & $adb -s $serial shell am force-stop com.google.android.googlesdksetup | Out-Null

    $imeApk = Join-Path $androidRoot "app\build\outputs\apk\debug\app-debug.apk"
    $testApk = Join-Path $androidRoot "attachment-test\build\outputs\apk\debug\attachment-test-debug.apk"
    Invoke-Checked $adb @("-s", $serial, "install", "-r", $imeApk) "安装输入法 APK 失败"
    Invoke-Checked $adb @("-s", $serial, "install", "-r", $testApk) "安装测试器 APK 失败"
    & $adb -s $serial shell pm clear com.shurufa.ime | Out-Null
    & $adb -s $serial shell pm clear com.shurufa.attachmenttest | Out-Null
    Invoke-Checked $adb @("-s", $serial, "shell", "ime", "enable", "com.shurufa.ime/.ShurufaImeService") "启用 Shurufa 输入法失败"
    Invoke-Checked $adb @("-s", $serial, "shell", "ime", "set", "com.shurufa.ime/.ShurufaImeService") "选择 Shurufa 输入法失败"
    Invoke-Checked $adb @("-s", $serial, "shell", "am", "broadcast", "-a", "com.shurufa.ime.DEBUG_INJECT_ATTACHMENTS", "-n", "com.shurufa.ime/.DebugAttachmentReceiver") "注入测试附件失败"
    Start-Sleep -Seconds 3

    $size = Get-ScreenSize $adb $serial
    $width = $size[0]
    $height = $size[1]

    Test-Delivery "rich-image" $adb $serial $width $height 645 2100 @("来源：commitContent", "声明 MIME：image/png", "解析 MIME：image/png")
    Test-Delivery "rich-file" $adb $serial $width $height 645 1900 @("来源：commitContent", "标签：shurufa-attachment-test.bin", "解析 MIME：application/octet-stream")
    Test-Delivery "jpeg-image" $adb $serial $width $height 1045 2100 @("来源：commitContent", "声明 MIME：image/jpeg", "解析 MIME：image/jpeg")
    Test-UndeclaredDelivery "undeclared-image" $adb $serial $width $height 1400 2100 @("来源：commitContent")

    Start-TestActivity $adb $serial
    Invoke-Tap $adb $serial $width $height 540 1825
    Wait-ForImeVisible $adb $serial
    Invoke-Tap $adb $serial $width $height 70 1650
    Start-Sleep -Seconds 1
    & $adb -s $serial logcat -c
    Invoke-Tap $adb $serial $width $height 300 2100
    Start-Sleep -Seconds 3
    $negativeLogs = Get-TestLogs $adb $serial
    if ($negativeLogs.Trim().Length -ne 0) {
        throw "普通文本负对照错误地接收了附件：`n$negativeLogs"
    }
    Invoke-Checked $adb @("-s", $serial, "shell", "uiautomator", "dump", "/sdcard/negative.xml") "导出负对照 UI 失败"
    Invoke-Checked $adb @("-s", $serial, "pull", "/sdcard/negative.xml", (Join-Path $artifacts "negative.xml")) "拉取负对照 UI 失败"
    $negativeUi = Get-Content -LiteralPath (Join-Path $artifacts "negative.xml") -Raw
    if ($negativeUi -notmatch "尚未收到附件") {
        throw "普通文本负对照结果状态异常"
    }
    Save-Screenshot $adb $serial "negative"
    Write-Host "通过：negative"

    Test-ClipboardFallback "clipboard-fallback" $adb $serial $width $height 1825 2100

    Write-Host ""
    Write-Host "Android 模拟器附件验收全部通过。"
    Write-Host "产物目录：$artifacts"
} finally {
    if ($startedEmulator -and -not $KeepEmulator) {
        & $adb -s $serial emu kill | Out-Null
    }
}
