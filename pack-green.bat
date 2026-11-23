@echo off
chcp 65001 >nul
cd /d "%~dp0"

REM Win7 SP1+ 绿色包必须用 1.77.2（避免 ProcessPrng）；与 Linux/macOS 的 stable 工具链分离
REM 桌面（默认）: pack-green.bat
REM 无 UI 服务器: set MEMO_SERVER=1 && pack-green.bat   或直接 pack-server.bat
set "MEMO_WIN_TOOLCHAIN=1.77.2"
rustup run %MEMO_WIN_TOOLCHAIN% rustc -V >nul 2>&1
if errorlevel 1 (
  echo [!] 未安装 %MEMO_WIN_TOOLCHAIN%，正在 rustup install ...
  rustup install %MEMO_WIN_TOOLCHAIN%
  if errorlevel 1 exit /b 1
)

set "CARGO_EXTRA="
set "OUT_NAME=memo.exe"
if /I "%MEMO_SERVER%"=="1" goto :server
if /I "%MEMO_NO_UI%"=="1" goto :server_deprecated
if /I "%MEMO_NO_TRAY%"=="1" goto :server_deprecated
goto :desktop

:server_deprecated
echo 提示: 请改用 MEMO_SERVER=1 或 pack-server.bat（MEMO_NO_UI/MEMO_NO_TRAY 已弃用）
set "MEMO_SERVER=1"

:server
set "CARGO_EXTRA=--no-default-features"
set "OUT_NAME=memo-server.exe"
echo [1/2] 服务器构建（无 UI，Rust %MEMO_WIN_TOOLCHAIN%）...
goto :build

:desktop
echo [1/2] 绿色免安装构建（桌面 UI，Rust %MEMO_WIN_TOOLCHAIN% + 静态 CRT）...

:build
REM Cargo.lock 须为 version=3；勿用新版 cargo update 写成 v4（1.77.2 无法解析）
rustup run %MEMO_WIN_TOOLCHAIN% cargo build -p memo-cli --release --locked %CARGO_EXTRA%
if errorlevel 1 (
  echo.
  echo 若报 lock file version 4：请把 Cargo.lock 首行改回 version = 3，或勿用 stable cargo 改写锁文件。
  exit /b 1
)

if not exist "dist" mkdir dist
copy /Y "target\release\memo.exe" "dist\%OUT_NAME%" >nul
if /I "%MEMO_SERVER%"=="1" (
  copy /Y "target\release\memo.exe" "dist\memo.exe" >nul
)

echo.
echo 完成: dist\%OUT_NAME%
if /I "%MEMO_SERVER%"=="1" (
  echo 说明: 无 UI 存储节点；首次设 MEMO_LOGIN_PASSWORD 后 memo --no-ui
  echo 亦已复制: dist\memo.exe
) else (
  echo 说明: 复制该文件即可使用；同网段自动发现需放行 TCP 端口 + UDP 17000
  echo 系统: Win7 SP1+ / 10 / 11 x64（须用本脚本 / %MEMO_WIN_TOOLCHAIN%）
)
echo.
echo 可选 Authenticode 签名（售卖分发建议）:
echo   设置 MEMO_SIGN_CERT 为证书指纹后取消下面注释:
REM if defined MEMO_SIGN_CERT (
REM   signtool sign /sha1 %MEMO_SIGN_CERT% /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 dist\%OUT_NAME%
REM )
echo.
dir dist\memo*.exe
