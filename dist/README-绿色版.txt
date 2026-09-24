分布式备忘录 绿色版 0.3.0
====================
复制 memo.exe 即可运行（Win7 SP1+ / 10 / 11 x64）。

社区版：仅本机加密存储（标题+正文）。
商业版：license.json → %AppData%\DistributedMemo\ 后重启，启用加密发现 + TLS 同步。
放行：TCP 监听端口（默认 7000）+ UDP 17000。
注意：0.3 与 0.2 节点不能互通。

构建：pack-green.bat
签名：设置 MEMO_SIGN_CERT=证书指纹 后重跑
