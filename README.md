# cf-tg-proxy-local

## 说明
- 这是一个 docker 部署的 telegram 代理桥，依赖的是 https://github.com/cmliu/edgetunnel
- 思路是，在国内任意一台服务器上，运行这个 docker 容器，然后对 edgetunnel 进行 socks5 / MTProto 包装
- 语言使用 rust，编译成 二进制文件，支持 arm64 / amd64 这 2 种cpu 结构
