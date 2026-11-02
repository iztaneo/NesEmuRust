@echo off
chcp 65001 >nul
title Instalador de NesEmuRust v1.0
echo ============================================================
echo        INSTALADOR DE NESEMURUST v1.0 (NES / Famicom)
echo ============================================================
echo.

set "INSTALL_DIR=%LOCALAPPDATA%\Programs\NesEmuRust"
set "SRC_DIR=%~dp0"

echo [1/4] Creando carpeta de instalacion en:
echo       "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%\rooms" mkdir "%INSTALL_DIR%\rooms"
if not exist "%INSTALL_DIR%\saves" mkdir "%INSTALL_DIR%\saves"

echo [2/4] Copiando ejecutable NesEmuRust.exe, partidas y documentacion...
if exist "%SRC_DIR%NesEmuRust.exe" (
    copy /Y "%SRC_DIR%NesEmuRust.exe" "%INSTALL_DIR%\NesEmuRust.exe" >nul
) else if exist "%SRC_DIR%target\release\nes_emu_rust.exe" (
    copy /Y "%SRC_DIR%target\release\nes_emu_rust.exe" "%INSTALL_DIR%\NesEmuRust.exe" >nul
)

if exist "%SRC_DIR%WIKI_ARQUITECTURA.md" (
    copy /Y "%SRC_DIR%WIKI_ARQUITECTURA.md" "%INSTALL_DIR%\WIKI_ARQUITECTURA.md" >nul
)

if exist "%SRC_DIR%saves\*.*" (
    xcopy /Y /Q "%SRC_DIR%saves\*.*" "%INSTALL_DIR%\saves\" >nul
)

if exist "%SRC_DIR%rooms\*.nes" (
    echo [3/4] Copiando catalogo de ROMs...
    xcopy /Y /Q "%SRC_DIR%rooms\*.*" "%INSTALL_DIR%\rooms\" >nul
)

echo [4/4] Creando accesos directos en el Escritorio y Menu Inicio...
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
    "$ws = New-Object -ComObject WScript.Shell; " ^
    "$desktop = [Environment]::GetFolderPath('Desktop'); " ^
    "$s1 = $ws.CreateShortcut((Join-Path $desktop 'NesEmuRust.lnk')); " ^
    "$s1.TargetPath = '%INSTALL_DIR%\NesEmuRust.exe'; " ^
    "$s1.WorkingDirectory = '%INSTALL_DIR%'; " ^
    "$s1.Description = 'Emulador NES / Famicom (Widescreen 16:9 + Audio 3D)'; " ^
    "$s1.Save(); " ^
    "$startMenu = [Environment]::GetFolderPath('Programs'); " ^
    "$s2 = $ws.CreateShortcut((Join-Path $startMenu 'NesEmuRust.lnk')); " ^
    "$s2.TargetPath = '%INSTALL_DIR%\NesEmuRust.exe'; " ^
    "$s2.WorkingDirectory = '%INSTALL_DIR%'; " ^
    "$s2.Description = 'Emulador NES / Famicom (Widescreen 16:9 + Audio 3D)'; " ^
    "$s2.Save();"

echo.
echo ============================================================
echo   INSTALACION COMPLETADA EXITOSAMENTE!
echo   - Ejecutable instalado en: %INSTALL_DIR%\NesEmuRust.exe
echo   - Acceso directo creado en tu Escritorio ("NesEmuRust")
echo   - Acceso directo creado en el Menu Inicio de Windows
echo ============================================================
echo.
pause
