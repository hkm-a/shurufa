@echo off
rem Build the Android APK: cross-compile the Rust JNI bridge, copy the
rem shared library into jniLibs, then run the Gradle debug build.
rem Prerequisites (one-time): JDK 17, Android SDK + NDK 28.0.13004108,
rem gradle 8.10.2 in %LOCALAPPDATA%\gradle-dist, rustup target
rem aarch64-linux-android, prebuilt librime in third_party\librime-android.
setlocal
set ROOT=%~dp0..
set JAVA_HOME=C:\Program Files\Microsoft\jdk-17.0.20.8-hotspot
set ANDROID_HOME=%LOCALAPPDATA%\Android\Sdk
set GRADLE=%LOCALAPPDATA%\gradle-dist\gradle-8.10.2\bin\gradle.bat

cd /d "%ROOT%"
cargo build --release --target aarch64-linux-android -p shurufa-rimejni || goto :fail
if not exist "%ROOT%\platforms\android\app\src\main\jniLibs\arm64-v8a" mkdir "%ROOT%\platforms\android\app\src\main\jniLibs\arm64-v8a"
copy /y "%ROOT%\target\aarch64-linux-android\release\libshurufa_rime.so" "%ROOT%\platforms\android\app\src\main\jniLibs\arm64-v8a\" >nul || goto :fail

cd /d "%ROOT%\platforms\android"
call "%GRADLE%" assembleDebug --console=plain || goto :fail
echo.
echo APK: %ROOT%\platforms\android\app\build\outputs\apk\debug\app-debug.apk
exit /b 0

:fail
echo [error] build failed
exit /b 1
