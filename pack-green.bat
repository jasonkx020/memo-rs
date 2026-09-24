@echo off
chcp 65001 >nul
cd /d "%~dp0"

echo [1/2] 绿色免安装构建（Rust 1.77.2 + 静态 CRT）...
cargo build -p memo-cli --release
if errorlevel 1 exit /b 1

if not exist "dist" mkdir dist
copy /Y "target\release\memo.exe" "dist\memo.exe" >nul

echo.
echo 完成: dist\memo.exe
echo 说明: 复制该文件即可使用；同网段自动发现需放行 TCP 端口 + UDP 17000
echo 系统: Win7 SP1+ / 10 / 11 x64（须用本脚本构建，勿用新版 rustc）
echo.
echo 可选 Authenticode 签名（售卖分发建议）:
echo   设置 MEMO_SIGN_CERT 为证书指纹后取消下面注释:
REM if defined MEMO_SIGN_CERT (
REM   signtool sign /sha1 %MEMO_SIGN_CERT% /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 dist\memo.exe
REM )
echo.
dir dist\memo.exe
