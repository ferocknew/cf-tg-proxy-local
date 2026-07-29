# cf-tg-proxy-local

## 说明
- docker 部署的 telegram 代理桥，基于 [edgetunnel](https://github.com/cmliu/edgetunnel)
- 在国内服务器运行此容器，对 edgetunnel 进行 socks5/MTProto 包装
- Rust 编译成二进制，支持 arm64/amd64（当前 CI 仅构建 amd64）

## 环境变量（.env）

代理本体：

| 变量 | 说明 | 默认值 |
|---|---|---|
| SUB_URL | edgetunnel 订阅地址 | - |
| TG_PROXY_TYPE | 代理类型（socks5） | socks5 |
| TG_PROXY_ADDR | 容器内监听地址 | 0.0.0.0 |
| TG_PROXY_PORT | 容器内监听端口 | 1080 |
| TG_PROXY_USER | socks5 用户名 | - |
| TG_PROXY_PASS | socks5 密码 | - |
| DOCKER_PORT | 宿主机对外映射端口 | - |

可选运行参数（不设则用代码默认值）：

| 变量 | 默认值 | 说明 |
|---|---|---|
| TG_HEARTBEAT_INTERVAL | 1800（秒） | 心跳探测间隔 |
| TG_SPEED_TIMEOUT | 3（秒） | 单节点测速超时 |
| TG_NODE_SLOW_THRESHOLD_MS | 500（毫秒） | 判定节点变慢的延迟阈值 |
| SHOES_CONFIG_PATH | /dev/shm/cf-tg-proxy/shoes.yaml | shoes 配置写入路径（须 tmpfs） |

## 部署

1. 在服务器创建目录，放入 `.env` 和 `compose.yaml`
2. 编辑 `.env`：填写 SUB_URL、认证凭据、DOCKER_PORT（宿主机对外端口）
3. 启动：`docker compose up -d`
4. 客户端连接：socks5 协议，地址 `服务器IP:DOCKER_PORT`

compose.yaml 结构：

```yaml
services:
  cf-tg-proxy:
    image: ghcr.io/ferocknew/cf-tg-proxy-local:0.1.0  # 或私服镜像
    container_name: cf-tg-proxy
    restart: unless-stopped
    env_file:
      - .env
    ports:
      - "${DOCKER_PORT}:${TG_PROXY_PORT}"
    shm_size: 64m
    logging:
      driver: json-file
      options:
        max-size: "10m"
        max-file: "3"
```

## 构建

- GitHub Actions 自动构建：push main 或打 tag 触发
- 镜像 tag 来自 VERSION 文件（当前 0.1.0），推送到 GHCR
- Tag 格式：无 v 前缀的版本号（如 0.1.0）
