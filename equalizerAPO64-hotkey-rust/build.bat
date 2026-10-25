@echo off
setlocal
cd /d "%~dp0"

cargo build --release --workspace
if errorlevel 1 goto :error

set STAGE=installer-output\files
if not exist "%STAGE%" mkdir "%STAGE%"

copy /Y target\release\EqualizerAPO.dll "%STAGE%\" >nul
copy /Y target\release\Editor.exe "%STAGE%\" >nul
copy /Y target\release\DeviceSelector.exe "%STAGE%\" >nul
copy /Y target\release\UpdateChecker.exe "%STAGE%\" >nul
copy /Y target\release\VoicemeeterClient.exe "%STAGE%\" >nul
copy /Y target\release\Benchmark.exe "%STAGE%\" >nul
copy /Y target\release\Setup.exe "%STAGE%\..\" >nul
xcopy /E /I /Y assets\setup-config "%STAGE%\config" >nul

echo Build finished. Installer payload is in installer-output.
goto :eof

:error
echo Build failed.
exit /b 1
