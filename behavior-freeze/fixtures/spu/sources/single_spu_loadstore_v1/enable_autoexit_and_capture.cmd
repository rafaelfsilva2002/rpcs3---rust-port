@echo off
REM ============================================================
REM single_spu_loadstore_v1 — RPCS3 trace capture (auto-exit)
REM ============================================================
REM
REM Identical workflow to sibling R5.11 fixtures' capture scripts.
REM Just double-click this in Explorer after `make`.

setlocal enabledelayedexpansion

set CFG=R:\bin\config\config.yml
if not exist "%CFG%" (
    echo [FAIL] Config not found at %CFG%
    pause
    exit /b 1
)

if not exist "%CFG%.bak_before_capture" copy "%CFG%" "%CFG%.bak_before_capture" >nul

powershell -NoProfile -Command "(Get-Content '%CFG%') -replace 'Exit RPCS3 when process finishes: false', 'Exit RPCS3 when process finishes: true' | Set-Content '%CFG%'"

echo [Step 1] Config patched
findstr /C:"Exit RPCS3 when process finishes" "%CFG%"
echo.

if exist "%TEMP%\single_spu_loadstore_v1.jsonl" del /q "%TEMP%\single_spu_loadstore_v1.jsonl"
if exist "%TEMP%\single_spu_loadstore_v1.images" rmdir /s /q "%TEMP%\single_spu_loadstore_v1.images"
if exist "%TEMP%\single_spu_loadstore_v1.jsonl.images" rmdir /s /q "%TEMP%\single_spu_loadstore_v1.jsonl.images"
echo [Step 2] Cleaned prior artifacts
echo.

set RPCS3_SPU_TRACE_JSONL=%TEMP%\single_spu_loadstore_v1.jsonl
echo [Step 3] Capturing to: %RPCS3_SPU_TRACE_JSONL%
echo          .self: %~dp0build\single_spu_loadstore_v1.self
echo.

echo [Step 4] Launching RPCS3...
"R:\bin\rpcs3.exe" --headless "%~dp0build\single_spu_loadstore_v1.self"

echo.
echo [Step 4 done] RPCS3 exited (code %ERRORLEVEL%)
echo.

if exist "%RPCS3_SPU_TRACE_JSONL%" (
    for %%I in ("%RPCS3_SPU_TRACE_JSONL%") do echo [OK] Trace: %%~fI ^(%%~zI bytes^)
) else (
    echo [FAIL] Trace file NOT found at %RPCS3_SPU_TRACE_JSONL%
)

if exist "%TEMP%\single_spu_loadstore_v1.jsonl.images" (
    echo [OK] Images dir: %TEMP%\single_spu_loadstore_v1.jsonl.images
    dir /b "%TEMP%\single_spu_loadstore_v1.jsonl.images"
) else if exist "%TEMP%\single_spu_loadstore_v1.images" (
    echo [OK] Images dir: %TEMP%\single_spu_loadstore_v1.images
    dir /b "%TEMP%\single_spu_loadstore_v1.images"
) else (
    echo [FAIL] No images dir found
)

echo.
pause
