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
echo 说明: 复制该文件即可使用；同步需商业授权；同网段发现需放行 TCP 端口 + UDP 17000
echo 系统: Win7 SP1+ / 10 / 11 x64（须用本脚本构建，勿用新版 rustc）
echo.

if defined MEMO_SIGN_CERT (
  echo [2/2] Authenticode 签名（证书指纹 %MEMO_SIGN_CERT%）...
  signtool sign /sha1 %MEMO_SIGN_CERT% /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 dist\memo.exe
  if errorlevel 1 (
    echo 签名失败，请检查 MEMO_SIGN_CERT 与 signtool PATH
    exit /b 1
  )
  echo 签名完成。
) else (
  echo [2/2] 跳过签名：未设置 MEMO_SIGN_CERT（证书 SHA1 指纹）。
  echo   售卖分发建议设置后重跑本脚本；需已安装 Windows SDK 的 signtool。
)

echo.
dir dist\memo.exe
