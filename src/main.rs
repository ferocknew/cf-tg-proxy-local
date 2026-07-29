//! cf-tg-proxy-local：TG 专用 socks5 代理桥（包装层）。
//! 职责：拉取 edgetunnel 订阅 → 测速选最优节点 → 生成 shoes 配置（写 tmpfs，0600）
//!       → 拉起 shoes 引擎 → 定时心跳。
//! 代理协议（socks5 入口 / VLESS over WS+TLS 出站）全部交给 shoes 引擎实现。
//!
//! 测速策略（省资源）：
//!   - 长间隔（默认 30 分钟）定时只探测【当前节点】1 次；
//!   - 仅当当前节点变慢（超阈值）/不可达时，才触发全量测速并切换最优。
//!
//! 环境变量（除 .env 里的 TG_PROXY_* / SUB_URL 外）：
//!   SHOES_BIN              shoes 二进制路径（默认 shoes；设为 none 则只跑选优不拉起）
//!   SHOES_CONFIG_PATH      生成的 YAML 路径（默认 /dev/shm/cf-tg-proxy/shoes.yaml）
//!   TG_HEARTBEAT_INTERVAL  心跳探测间隔秒数（默认 1800 = 30 分钟）
//!   TG_SPEED_TIMEOUT       单节点测速超时秒数（默认 3）
//!   TG_NODE_SLOW_THRESHOLD_MS  当前节点判定变慢的延迟阈值（默认 1000）

mod config;
mod shoes_config;
mod speed_test;
mod subscription;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;
use crate::subscription::VlessNode;

const DEFAULT_HEARTBEAT_SECS: u64 = 1800; // 30 分钟
const DEFAULT_SPEED_TIMEOUT_SECS: u64 = 3;
const DEFAULT_SLOW_THRESHOLD_MS: u64 = 500;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_tracing();

    let cfg = Config::from_env();
    info!(config = ?cfg, "配置已加载");

    let shoes_bin = std::env::var("SHOES_BIN").unwrap_or_else(|_| "shoes".to_string());
    let config_path = std::env::var("SHOES_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/dev/shm/cf-tg-proxy/shoes.yaml"));
    let interval = Duration::from_secs(env_u64("TG_HEARTBEAT_INTERVAL", DEFAULT_HEARTBEAT_SECS));
    let speed_timeout = Duration::from_secs(env_u64("TG_SPEED_TIMEOUT", DEFAULT_SPEED_TIMEOUT_SECS));
    let slow_threshold =
        Duration::from_millis(env_u64("TG_NODE_SLOW_THRESHOLD_MS", DEFAULT_SLOW_THRESHOLD_MS));

    // 1. 初始选优 + 生成配置
    let best = select_best_node(&cfg, speed_timeout)
        .await
        .context("初始选优失败")?;
    write_yaml_atomic(
        &config_path,
        &shoes_config::to_yaml(&cfg, std::slice::from_ref(&best))?,
    )?;
    info!(path = %config_path.display(), "已生成初始 shoes 配置");
    let current = Arc::new(Mutex::new(best));

    // 2. 心跳循环（后台）
    {
        let cfg = cfg.clone();
        let path = config_path.clone();
        let current = current.clone();
        tokio::spawn(async move {
            heartbeat_loop(&cfg, &path, speed_timeout, slow_threshold, interval, current).await;
        });
    }

    // 3. 拉起 shoes，或仅运行选优（SHOES_BIN=none）
    if shoes_bin.is_empty() || shoes_bin == "none" {
        info!("SHOES_BIN 未配置，仅运行选优 + 心跳（不拉起 shoes）。Ctrl-C 退出。");
        tokio::signal::ctrl_c().await.ok();
        return Ok(());
    }
    let mut shoes = spawn_shoes(&shoes_bin, &config_path)?;
    let status = shoes.wait().await.context("等待 shoes 退出失败")?;
    if !status.success() {
        return Err(anyhow!("shoes 异常退出: {status}"));
    }
    Ok(())
}

/// 心跳：长间隔只探测当前节点；变慢/不通才全量测速切换。
async fn heartbeat_loop(
    cfg: &Config,
    path: &Path,
    speed_timeout: Duration,
    slow_threshold: Duration,
    interval: Duration,
    current: Arc<Mutex<VlessNode>>,
) {
    loop {
        tokio::time::sleep(interval).await;
        let cur = current.lock().unwrap().clone();

        // 先只探测当前节点（1 次 TCP connect）
        match speed_test::test_single(&cur, speed_timeout).await {
            Some(rtt) if rtt <= slow_threshold => {
                info!(
                    addr = %cur.address,
                    rtt_ms = rtt.as_millis() as u64,
                    "当前节点健康，跳过全量测速"
                );
                continue;
            }
            Some(rtt) => warn!(
                addr = %cur.address,
                rtt_ms = rtt.as_millis() as u64,
                "当前节点变慢，触发全量测速"
            ),
            None => warn!(addr = %cur.address, "当前节点不可达，触发全量测速"),
        }

        // 全量测速选优
        match select_best_node(cfg, speed_timeout).await {
            Ok(new_best) => {
                if new_best.signature() == cur.signature() {
                    warn!("全量测速后最优仍是当前节点，暂不切换");
                    continue;
                }
                match shoes_config::to_yaml(cfg, std::slice::from_ref(&new_best)) {
                    Ok(yaml) => match write_yaml_atomic(path, &yaml) {
                        Ok(()) => {
                            info!("节点切换：{} -> {}", cur.signature(), new_best.signature());
                            *current.lock().unwrap() = new_best;
                        }
                        Err(e) => warn!("重写配置失败: {e}"),
                    },
                    Err(e) => warn!("生成配置失败: {e}"),
                }
            }
            Err(e) => warn!("全量测速失败: {e}"),
        }
    }
}

/// 拉取订阅 + 全量测速 + 选最优。
async fn select_best_node(cfg: &Config, timeout: Duration) -> anyhow::Result<VlessNode> {
    let nodes = subscription::fetch_nodes(cfg).await?;
    info!("拉取到 {} 个节点，开始全量测速", nodes.len());
    let results = speed_test::test_all(&nodes, timeout).await;
    let best_idx =
        speed_test::pick_best(&results).ok_or_else(|| anyhow!("所有节点均不可达"))?;
    let best_rtt = results
        .iter()
        .find(|r| r.index == best_idx)
        .and_then(|r| r.rtt);
    let best = nodes.into_iter().nth(best_idx).expect("best_idx 有效");
    info!(
        addr = %best.address,
        port = best.port,
        name = %best.name,
        rtt_ms = best_rtt.map(|d| d.as_millis() as u64).unwrap_or(0),
        "选出最优节点"
    );
    Ok(best)
}

/// 拉起 shoes 子进程。
fn spawn_shoes(bin: &str, config: &Path) -> anyhow::Result<tokio::process::Child> {
    let child = Command::new(bin)
        .arg(config)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("拉起 shoes({bin}) 失败"))?;
    info!(bin, config = %config.display(), "shoes 已启动");
    Ok(child)
}

/// 原子写 YAML：先写 .tmp 并设 0600，再 rename，避免 shoes 读到半截。
fn write_yaml_atomic(path: &Path, yaml: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, yaml).with_context(|| format!("写 {} 失败", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).with_context(|| format!("rename 到 {} 失败", path.display()))?;
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
