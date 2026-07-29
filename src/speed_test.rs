//! 节点测速：对所有候选节点并发测 TCP connect RTT，选最快的。
//! MVP 测本机到节点优选 IP 的 TCP 连接延迟（CF 优选场景下这是主导因素）。
//! 端到端（经节点连 TG IP 段）留作后续增强。

use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::task::JoinSet;

use crate::subscription::VlessNode;

/// 单个节点的测速结果。
#[derive(Debug, Clone)]
pub struct SpeedResult {
    /// 在原节点列表中的下标
    pub index: usize,
    /// TCP connect 往返延迟；None 表示连接失败或超时
    pub rtt: Option<Duration>,
}

/// 并发对所有节点测 TCP connect RTT。
pub async fn test_all(nodes: &[VlessNode], timeout: Duration) -> Vec<SpeedResult> {
    let mut set: JoinSet<(usize, Option<Duration>)> = JoinSet::new();
    for (i, n) in nodes.iter().enumerate() {
        let addr = n.address.clone();
        let port = n.port;
        set.spawn(async move { (i, test_one(addr, port, timeout).await) });
    }
    let mut results = Vec::with_capacity(set.len());
    while let Some(res) = set.join_next().await {
        if let Ok((i, rtt)) = res {
            results.push(SpeedResult { index: i, rtt });
        }
    }
    results
}

/// 测单个地址的 TCP connect 延迟。
async fn test_one(addr: String, port: u16, timeout: Duration) -> Option<Duration> {
    let start = Instant::now();
    let conn = tokio::time::timeout(timeout, TcpStream::connect((addr, port))).await;
    match conn {
        Ok(Ok(_stream)) => Some(start.elapsed()),
        _ => None,
    }
}

/// 测单个节点的 TCP connect 延迟（供心跳探测当前节点使用）。
pub async fn test_single(node: &VlessNode, timeout: Duration) -> Option<Duration> {
    test_one(node.address.clone(), node.port, timeout).await
}

/// 从测速结果中选出延迟最低的可达节点下标；全部不可达返回 None。
pub fn pick_best(results: &[SpeedResult]) -> Option<usize> {
    results
        .iter()
        .filter_map(|r| r.rtt.map(|rtt| (r.index, rtt)))
        .min_by_key(|(_, rtt)| *rtt)
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_best_选延迟最低() {
        let results = vec![
            SpeedResult { index: 0, rtt: None },
            SpeedResult { index: 1, rtt: Some(Duration::from_millis(100)) },
            SpeedResult { index: 2, rtt: Some(Duration::from_millis(50)) },
            SpeedResult { index: 3, rtt: Some(Duration::from_millis(200)) },
        ];
        assert_eq!(pick_best(&results), Some(2));
    }

    #[test]
    fn pick_best_全不可达返回none() {
        let results = vec![
            SpeedResult { index: 0, rtt: None },
            SpeedResult { index: 1, rtt: None },
        ];
        assert_eq!(pick_best(&results), None);
    }

    #[test]
    fn pick_best_唯一可达即选它() {
        let results = vec![
            SpeedResult { index: 0, rtt: None },
            SpeedResult { index: 5, rtt: Some(Duration::from_secs(2)) },
        ];
        assert_eq!(pick_best(&results), Some(5));
    }
}
