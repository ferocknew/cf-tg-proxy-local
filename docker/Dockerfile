# syntax=docker/dockerfile:1.6

########## builder：编译包装层 + shoes 引擎 ##########
FROM rust:1-slim AS builder

# shoes 依赖 ring/rustls，需要 C 工具链
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# 先单独编译 shoes（依赖多，独立一层利于缓存；cache mount 加速）
# 用 cargo install 而非下载 release：shoes 没有 linux/arm64 预编译包
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install --locked shoes

# 编译我们的包装层
COPY Cargo.toml ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release \
    && cp target/release/cf-tg-proxy-local /cf-tg-proxy-local

########## runtime：最小镜像 ##########
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 两个二进制：包装层 + shoes
COPY --from=builder /usr/local/cargo/bin/shoes /usr/local/bin/shoes
COPY --from=builder /cf-tg-proxy-local /usr/local/bin/cf-tg-proxy-local

# shoes 二进制路径 + 配置写到 tmpfs（内存，不落持久盘，防泄露订阅）
ENV SHOES_BIN=/usr/local/bin/shoes
ENV SHOES_CONFIG_PATH=/dev/shm/cf-tg-proxy/shoes.yaml
# 心跳 / 测速默认参数（可被 docker run -e 覆盖）
ENV TG_HEARTBEAT_INTERVAL=1800
ENV TG_SPEED_TIMEOUT=3
# 当前节点 RTT 超过 500ms 即视为变慢，触发全量测速换节点
ENV TG_NODE_SLOW_THRESHOLD_MS=500

EXPOSE 1080

# 入口 = 包装层：它会拉起 shoes 并做心跳
ENTRYPOINT ["/usr/local/bin/cf-tg-proxy-local"]
