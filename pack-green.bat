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
set "OUT_NAME=mijian.exe"
if /I "%MEMO_SERVER%"=="1" goto :server
if /I "%MEMO_NO_UI%"=="1" goto :server_deprecated
if /I "%MEMO_NO_TRAY%"=="1" goto :server_deprecated
goto :desktop

:server_deprecated
echo 提示: 请改用 MEMO_SERVER=1 或 pack-server.bat（MEMO_NO_UI/MEMO_NO_TRAY 已弃用）
set "MEMO_SERVER=1"

:server
set "CARGO_EXTRA=--no-default-features"
set "OUT_NAME=mijian-server.exe"
echo [1/2] 服务器构建（无 UI，Rust %MEMO_WIN_TOOLCHAIN%）...
goto :build

:desktop
echo [1/2] 绿色免安装构建（桌面 UI，Rust %MEMO_WIN_TOOLCHAIN% + 静态 CRT）...

:build
REM Cargo.lock 须为 version=3；勿用新版 cargo update 写成 v4（1.77.2 无法解析）
rustup run %MEMO_WIN_TOOLCHAIN% cargo build -p mijian-cli --release --locked %CARGO_EXTRA%
if errorlevel 1 (
  echo.
  echo 若报 lock file version 4：请把 Cargo.lock 首行改回 version = 3，或勿用 stable cargo 改写锁文件。
  exit /b 1
)

if not exist "dist" mkdir dist
copy /Y "target\release\mijian.exe" "dist\%OUT_NAME%" >nul
if /I "%MEMO_SERVER%"=="1" (
  copy /Y "target\release\mijian.exe" "dist\mijian.exe" >nul
)

REM 从 workspace Cargo.toml 写入绿色版说明首行版本（与界面 / 文件属性一致）
for /f "usebackq tokens=3 delims= " %%V in (`findstr /B /C:"version = " Cargo.toml`) do (
  set "APP_VER=%%~V"
  goto :got_ver
)
:got_ver
set "APP_VER=%APP_VER:"=%"
(
  echo 密笺（Mijian）绿色版 %APP_VER%
  echo ====================
  echo 复制 mijian.exe 即可运行（Win7 SP1+ / 10 / 11 x64）。
  echo.
  echo 社区版：仅本机加密存储（标题+正文）。
  echo 商业版：license.json → %%AppData%%\Mijian\ 后重启，启用加密发现 + TLS 同步。
  echo 放行：TCP 监听端口（默认 7000）+ UDP 17000。
  echo.
  echo 构建：pack-green.bat
  echo 签名：设置 MEMO_SIGN_CERT=证书指纹 后重跑
) > "dist\README-绿色版.txt"

echo.
echo 完成: dist\%OUT_NAME%  （版本 %APP_VER%）
if /I "%MEMO_SERVER%"=="1" (
  echo 说明: 无 UI 存储节点；首次设 MEMO_LOGIN_PASSWORD 后 mijian --no-ui
  echo 亦已复制: dist\mijian.exe
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
dir dist\mijian*.exe
