@echo off
setlocal EnableDelayedExpansion

REM  Deploys the built binary into Nigel's own working install.
REM
REM  Run it from an ELEVATED prompt -- C:\Program Files is not writable otherwise, and this refuses
REM  to start rather than failing halfway with an access-denied on the copy itself.
REM
REM      deploy.cmd                       deploys to C:\Program Files\TailHawk
REM      deploy.cmd "D:\Some\Other\Dir"   deploys somewhere else
REM
REM  It does not build. Build first, and check the gate is green:
REM      cargo build --release -p tailhawk
REM      tools\check.sh

set "SOURCE=%~dp0target\release\tailhawk.exe"
set "TARGET=%~1"
if "%TARGET%"=="" set "TARGET=C:\Program Files\TailHawk"

echo.
echo Tailhawk deploy
echo   from  %SOURCE%
echo   to    %TARGET%
echo.

REM  fltmc needs administrator, so its exit code is the cheapest elevation test there is.
fltmc >nul 2>&1
if errorlevel 1 (
    echo   NOT ELEVATED.
    echo.
    echo   Right-click Command Prompt or Windows Terminal, "Run as administrator",
    echo   then run this again. Nothing has been changed.
    exit /b 1
)

if not exist "%SOURCE%" (
    echo   The binary is not there. Build it first:
    echo.
    echo       cargo build --release -p tailhawk
    echo.
    exit /b 1
)

if not exist "%TARGET%" (
    echo   Creating %TARGET%
    mkdir "%TARGET%" || (
        echo   Could not create it. Nothing has been changed.
        exit /b 1
    )
)

REM  Both versions before the copy, so the line printed at the end says what replaced what rather
REM  than just "done". The PE version resource is the authority -- see crates\tailhawk\build.rs.
set "WAS=none"
if exist "%TARGET%\tailhawk.exe" (
    for /f "usebackq delims=" %%v in (`powershell -NoProfile -Command "(Get-Item '%TARGET%\tailhawk.exe').VersionInfo.FileVersion"`) do set "WAS=%%v"
)
for /f "usebackq delims=" %%v in (`powershell -NoProfile -Command "(Get-Item '%SOURCE%').VersionInfo.FileVersion"`) do set "NOW=%%v"

copy /y "%SOURCE%" "%TARGET%\tailhawk.exe" >nul
if errorlevel 1 (
    echo.
    echo   The copy failed. The usual reason is that the deployed Tailhawk is still running --
    echo   Windows locks a running executable. Close it and run this again.
    echo.
    exit /b 1
)

echo   Deployed %NOW%   ^(was %WAS%^)
echo.
exit /b 0
