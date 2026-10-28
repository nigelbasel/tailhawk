@echo off
setlocal EnableDelayedExpansion

REM  Deploys the latest GitHub release into Nigel's own working install.
REM
REM  Run it from an ELEVATED prompt -- C:\Program Files is not writable otherwise, and this refuses
REM  to start rather than failing halfway with an access-denied on the copy itself.
REM
REM      deploy.cmd                       fetches the latest release, deploys to C:\Program Files\TailHawk
REM      deploy.cmd "D:\Some\Other\Dir"   deploys it somewhere else
REM      deploy.cmd local                 deploys the locally built target\release\tailhawk.exe instead
REM
REM  The release is whatever the last green push to master published -- see the release job in
REM  .github\workflows\ci.yml. The asset is named tailhawk-x64.exe up there and tailhawk.exe here,
REM  and its SHA-256 is checked against the release's SHA256SUMS.txt before anything is replaced.
REM  Neither the download nor the check needs a credential: the repository is public.
REM
REM  Every failure goes through :failed at the bottom. An `exit /b` inside a parenthesised block
REM  set ERRORLEVEL but came out of `cmd /c deploy.cmd` as 0; a goto to a top-level exit does not.

set "LATEST=https://github.com/nigelbasel/tailhawk/releases/latest/download"
set "LOCAL=%~dp0target\release\tailhawk.exe"
set "FETCHED=%TEMP%\tailhawk-deploy\tailhawk-x64.exe"
set "SUMS=%TEMP%\tailhawk-deploy\SHA256SUMS.txt"

set "MODE=release"
set "TARGET=%~1"
if /i "%TARGET%"=="local" (
    set "MODE=local"
    set "TARGET=%~2"
)
if "%TARGET%"=="" set "TARGET=C:\Program Files\TailHawk"

echo.
echo Tailhawk deploy
if "%MODE%"=="local" (
    echo   from  %LOCAL%
) else (
    echo   from  %LATEST%/tailhawk-x64.exe
)
echo   to    %TARGET%\tailhawk.exe
echo.

REM  fltmc needs administrator, so its exit code is the cheapest elevation test there is.
fltmc >nul 2>&1
if errorlevel 1 (
    echo   NOT ELEVATED.
    echo.
    echo   Right-click Command Prompt or Windows Terminal, "Run as administrator",
    echo   then run this again.
    goto :failed
)

if "%MODE%"=="local" (
    set "SOURCE=%LOCAL%"
    if not exist "!SOURCE!" (
        echo   The binary is not there. Build it first:
        echo.
        echo       cargo build --release -p tailhawk
        echo.
        goto :failed
    )
) else (
    if not exist "%TEMP%\tailhawk-deploy" mkdir "%TEMP%\tailhawk-deploy"
    del /q "%FETCHED%" "%SUMS%" >nul 2>&1
    echo   Fetching the latest release...
    curl.exe -fsSL -o "%FETCHED%" "%LATEST%/tailhawk-x64.exe"
    if errorlevel 1 (
        echo   The download failed.
        goto :failed
    )
    curl.exe -fsSL -o "%SUMS%" "%LATEST%/SHA256SUMS.txt"
    if errorlevel 1 (
        echo   The checksum list did not download.
        goto :failed
    )
    set "HASH="
    for /f "usebackq delims=" %%h in (`powershell -NoProfile -Command "(Get-FileHash -Algorithm SHA256 '%FETCHED%').Hash.ToLower()"`) do set "HASH=%%h"
    if not defined HASH (
        echo   The download could not be hashed, so it was not checked.
        goto :failed
    )
    findstr /i /c:"!HASH!  tailhawk-x64.exe" "%SUMS%" >nul
    if errorlevel 1 (
        echo   The download's SHA-256 !HASH! is not the one the release lists for tailhawk-x64.exe.
        goto :failed
    )
    echo   SHA-256 verified against the release's SHA256SUMS.txt
    set "SOURCE=%FETCHED%"
)

if not exist "%TARGET%" (
    echo   Creating %TARGET%
    mkdir "%TARGET%"
    if errorlevel 1 (
        echo   Could not create it.
        goto :failed
    )
)

REM  Both versions before the copy, so the line printed at the end says what replaced what rather
REM  than just "done". The PE version resource is the authority -- see crates\tailhawk\build.rs.
set "WAS=none"
if exist "%TARGET%\tailhawk.exe" (
    for /f "usebackq delims=" %%v in (`powershell -NoProfile -Command "(Get-Item '%TARGET%\tailhawk.exe').VersionInfo.FileVersion"`) do set "WAS=%%v"
)
for /f "usebackq delims=" %%v in (`powershell -NoProfile -Command "(Get-Item '!SOURCE!').VersionInfo.FileVersion"`) do set "NOW=%%v"

copy /y "!SOURCE!" "%TARGET%\tailhawk.exe" >nul
if errorlevel 1 (
    echo.
    echo   The copy failed. The usual reason is that the deployed Tailhawk is still running --
    echo   Windows locks a running executable. Close it and run this again.
    goto :failed
)

echo   Deployed %NOW%   ^(was %WAS%^)
echo.
exit /b 0

:failed
echo   Nothing has been changed.
echo.
exit /b 1
