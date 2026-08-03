@echo off
rem Start the clipboard listener under the supervisor (daemon lifecycle).
rem The supervisor enforces a single instance (named mutex), auto-restarts a
rem crashed worker with backoff, and exposes unified status/stop:
rem   shurufa-host status | stop
rem No more taskkill: a stale supervisor is refused by the singleton lock
rem (use "stop" first, or just call status to see what is running).
setlocal
set EXE=%~dp0..\target\debug\shurufa-host.exe
if not exist "%EXE%" (
    echo [error] build first: cargo build -p shurufa-host
    pause
    exit /b 1
)
start "shurufa-host-supervisor" /min "%EXE%" supervise
echo Supervisor started (watchdog for the clipboard listener).
echo Status: "%EXE%" status   Stop: "%EXE%" stop
echo Log file: %%TEMP%%\shurufa-host.log
endlocal
