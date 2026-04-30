@echo off
REM ============================================================
REM single_spu_mailbox_v1 — RPCS3 trace capture (with auto-exit)
REM ============================================================
REM
REM Step 1: Patches R:\bin\config\config.yml so RPCS3 auto-exits
REM         when our program calls _sys_process_exit. Without this,
REM         the trace writer's destructor never runs and the JSONL
REM         stays at 0 bytes.
REM
REM Step 2: Cleans prior trace + side-files.
REM Step 3: Sets RPCS3_SPU_TRACE_JSONL env var.
REM Step 4: Launches RPCS3 with the .self.
REM Step 5: After RPCS3 exits, verifies trace artifacts.
REM
REM Just double-click this in Explorer.

setlocal enabledelayedexpansion

REM === Step 1: enable Exit-RPCS3-when-process-finishes in config ===
set CFG=R:\bin\config\config.yml
if not exist "%CFG%" (
    echo [FAIL] Config not found at %CFG%
    pause
    exit /b 1
)

REM Backup once if not already
if not exist "%CFG%.bak_before_capture" copy "%CFG%" "%CFG%.bak_before_capture" >nul

powershell -NoProfile -Command "(Get-Content '%CFG%') -replace 'Exit RPCS3 when process finishes: false', 'Exit RPCS3 when process finishes: true' | Set-Content '%CFG%'"

echo [Step 1] Config patched: 'Exit RPCS3 when process finishes' = true
findstr /C:"Exit RPCS3 when process finishes" "%CFG%"
echo.

REM === Step 2: clean prior trace artifacts ===
if exist "%TEMP%\single_spu_mailbox_v1.jsonl" del /q "%TEMP%\single_spu_mailbox_v1.jsonl"
if exist "%TEMP%\single_spu_mailbox_v1.images" rmdir /s /q "%TEMP%\single_spu_mailbox_v1.images"
if exist "%TEMP%\single_spu_mailbox_v1.jsonl.images" rmdir /s /q "%TEMP%\single_spu_mailbox_v1.jsonl.images"
echo [Step 2] Cleaned prior trace artifacts
echo.

REM === Step 3: set env var ===
set RPCS3_SPU_TRACE_JSONL=%TEMP%\single_spu_mailbox_v1.jsonl
echo [Step 3] Capturing trace to: %RPCS3_SPU_TRACE_JSONL%
echo          .self: %~dp0build\single_spu_mailbox_v1.self
echo.

REM === Step 4: launch RPCS3 (will auto-exit when program finishes) ===
echo [Step 4] Launching RPCS3...
"R:\bin\rpcs3.exe" --headless "%~dp0build\single_spu_mailbox_v1.self"

echo.
echo [Step 4 done] RPCS3 exited (code %ERRORLEVEL%)
echo.

REM === Step 5: verify trace artifacts ===
echo === Verifying trace artifacts ===
if exist "%RPCS3_SPU_TRACE_JSONL%" (
    for %%I in ("%RPCS3_SPU_TRACE_JSONL%") do echo [OK] Trace file exists: %%~fI ^(%%~zI bytes^)
) else (
    echo [FAIL] Trace file NOT found at %RPCS3_SPU_TRACE_JSONL%
)

REM Side-files may be in either of two paths depending on writer convention
if exist "%TEMP%\single_spu_mailbox_v1.jsonl.images" (
    echo [OK] Images dir: %TEMP%\single_spu_mailbox_v1.jsonl.images
    dir /b "%TEMP%\single_spu_mailbox_v1.jsonl.images"
) else if exist "%TEMP%\single_spu_mailbox_v1.images" (
    echo [OK] Images dir: %TEMP%\single_spu_mailbox_v1.images
    dir /b "%TEMP%\single_spu_mailbox_v1.images"
) else (
    echo [FAIL] No images dir found
)

echo.
pause
