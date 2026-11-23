@echo off
chcp 65001 >nul
cd /d "%~dp0"
REM Windows 无 UI 存储节点包（等价于 set MEMO_SERVER=1 && pack-green.bat）
set "MEMO_SERVER=1"
call "%~dp0pack-green.bat"
