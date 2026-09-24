分布式备忘录 绿色版 0.2.0
====================
复制 memo.exe 即可运行（Win7 SP1+ / 10 / 11 x64）。

社区版：仅本机加密存储。
商业版：将 license.json 放到 %AppData%\DistributedMemo\ 后重启，可启用局域网同步。
同步需放行：TCP 监听端口（默认 7000）+ UDP 17000。

构建：仓库根目录执行 pack-green.bat
签名：设置 MEMO_SIGN_CERT=证书指纹 后重跑 pack-green.bat
