@echo off
REM ============================================================
REM single_spu_mailbox_v1 — RPCS3 trace capture wrapper
REM ============================================================
REM Just double-click this file in Explorer.
REM RPCS3 will open, run the .self (~2s of mailbox handshake),
REM and exit. The trace will be at %TEMP%\single_spu_mailbox_v1.jsonl

setlocal

REM Clean prior trace + side-files
if exist "%TEMP%\single_spu_mailbox_v1.jsonl" del /q "%TEMP%\single_spu_mailbox_v1.jsonl"
if exist "%TEMP%\single_spu_mailbox_v1.images" rmdir /s /q "%TEMP%\single_spu_mailbox_v1.images"

REM Set env var that RPCS3 R5.9c+R5.9e.3 writer reads
set RPCS3_SPU_TRACE_JSONL=%TEMP%\single_spu_mailbox_v1.jsonl

echo Capturing trace to: %RPCS3_SPU_TRACE_JSONL%
echo Self: %~dp0build\single_spu_mailbox_v1.self
echo.
echo Launching RPCS3 (will run ~2s + auto-exit)...

REM %~dp0 = directory of this .cmd file (with trailing \)
"R:\bin\rpcs3.exe" --headless "%~dp0build\single_spu_mailbox_v1.self"

echo.
echo RPCS3 exited (code %ERRORLEVEL%)
echo.
echo === Verifying trace artifacts ===
if exist "%RPCS3_SPU_TRACE_JSONL%" (
    echo [OK] Trace file: %RPCS3_SPU_TRACE_JSONL%
    for %%I in ("%RPCS3_SPU_TRACE_JSONL%") do echo      Size: %%~zI bytes
) else (
    echo [FAIL] Trace file NOT created at %RPCS3_SPU_TRACE_JSONL%
    echo        Possible causes:
    echo        - RPCS3 build does not include R5.9c+R5.9e.3 trace writer
    echo        - The .self failed to load ^(check R:\bin\log\RPCS3.log^)
    echo        - Env var RPCS3_SPU_TRACE_JSONL was not picked up
)

if exist "%TEMP%\single_spu_mailbox_v1.images" (
    echo [OK] Images dir exists: %TEMP%\single_spu_mailbox_v1.images
    dir /b "%TEMP%\single_spu_mailbox_v1.images"
) else (
    echo [WARN] Images dir NOT created
)

echo.
pause
