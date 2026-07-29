# cf-tg-proxy-local

## 项目概述
TG 专用 socks5 代理桥，基于 Rust + shoes 引擎，Docker 容器部署。
包装层拉 edgetunnel 订阅 → 测速选优 → 生成 shoes 配置 → 拉起 shoes 子进程 → 定时心跳。
shoes 引擎提供 socks5/VLESS-over-WS+TLS 代理。

## 架构与构建
- Dockerfile 多阶段：builder(rust:1-slim, Debian trixie) + runtime(debian:trixie-slim)
- builder 和 runtime 必须同 Debian 大版本，否则 glibc 不兼容（GLIBC_2.39 not found）
- 当前仅 linux/amd64；shoes 通过 cargo install 编译（无预编译包）
- shoes 依赖 ring/rustls，需要 C 工具链（build-essential/pkg-config）

## .env 配置（唯一真相源）
- 所有配置（订阅/代理/端口/认证/运行参数）全部来自 .env
- compose 通过 env_file 注入容器，不在 environment 段写业务参数
- 运行参数（TG_HEARTBEAT_INTERVAL 等）可选，不加则用代码默认值

## 端口设计（两个变量分工）
- TG_PROXY_PORT：容器内 rust/shoes 监听端口（默认 1080，写在 .env）
- DOCKER_PORT：宿主机对外映射端口（写在 .env）
- compose ports 用 ${DOCKER_PORT}:${TG_PROXY_PORT} 变量插值，不写死数字
- Dockerfile EXPOSE 1080 与 TG_PROXY_PORT 一致

## compose 规范
- 所有配置通过 env_file:.env 注入，environment 段不写业务参数
- ports 用变量插值：${DOCKER_PORT}:${TG_PROXY_PORT}
- shm_size:64m（shoes 配置写 tmpfs 防泄露订阅机密）
- shoes 配置必须只写 tmpfs 并设 0600 权限

## CI 与发布（.github/workflows/docker.yml）
- 触发：push main + tag（兼容 v* 前缀和无 v 前缀数字 tag，如 0.1.0）
- 镜像 tag 来自 VERSION 文件（type=raw），不依赖 metadata-action 的 semver/sha
- 推送 GHCR；私服可通过 Nexus proxy 上游自动代理

## 历史踩坑记录
- GLIBC 不兼容：builder(trixie,glibc2.40) vs runtime(bookworm,glibc2.36) → runtime 改 trixie-slim
- 端口搞反：曾误用 TG_PROXY_PORT 做宿主机端口 → 正确分工 TG_PROXY_PORT=容器内, DOCKER_PORT=宿主机
- compose environment 覆盖：曾在 environment 写死端口 → 改为纯 env_file，全部来自 .env
- VERSION 文件：0.1.0 tag 来自 VERSION(type=raw)，镜像 tag 不再用 semver/sha
