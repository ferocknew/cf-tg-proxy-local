//! 订阅解析：拉取 SUB_URL，base64 解码后按行解析 VLESS 节点。

use crate::config::Config;
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use percent_encoding::percent_decode_str;
use url::Url;

/// 单个 VLESS 节点（ws + tls）。
#[derive(Clone, Debug)]
pub struct VlessNode {
    pub uuid: String,
    /// 优选 IP / 连接地址
    pub address: String,
    pub port: u16,
    /// TLS SNI
    pub sni: String,
    /// WebSocket Host 头
    pub host: String,
    /// WebSocket 路径
    pub path: String,
    /// uTLS 指纹（MVP 暂不使用，预留）
    #[allow(dead_code)]
    pub fp: Option<String>,
    /// 节点名称（已 URL 解码）
    pub name: String,
}

impl VlessNode {
    /// 节点签名（address:port），用于检测最优节点是否变化。
    pub fn signature(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }
}

/// 多节点的稳定签名：先按各节点 signature 排序，再逗号拼接。
/// 排序保证"集合相同但顺序不同"（RTT 抖动导致 top-N 顺序变）时签名一致，
/// 避免心跳误判 top-N 变化而无谓重写配置、触发 shoes 频繁热重载。
pub fn nodes_signature(nodes: &[VlessNode]) -> String {
    let mut sigs: Vec<String> = nodes.iter().map(|n| n.signature()).collect();
    sigs.sort();
    sigs.join(",")
}

/// 拉取订阅并解析为节点列表。
pub async fn fetch_nodes(cfg: &Config) -> Result<Vec<VlessNode>> {
    if cfg.sub_url.is_empty() {
        return Err(anyhow!("SUB_URL 未配置"));
    }
    let resp = reqwest::get(&cfg.sub_url)
        .await
        .context("拉取订阅失败")?
        .error_for_status()
        .context("订阅返回非 2xx")?;
    let body = resp.text().await.context("读取订阅正文失败")?;
    parse_subscription(&body)
}

/// 解析订阅正文：支持 base64 编码与明文两种格式。
pub fn parse_subscription(body: &str) -> Result<Vec<VlessNode>> {
    let trimmed = body.trim();
    let decoded: String = if trimmed.starts_with("vless://") {
        trimmed.to_string()
    } else {
        let cleaned: Vec<u8> = trimmed
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let bytes = STANDARD
            .decode(&cleaned)
            .context("订阅 base64 解码失败")?;
        String::from_utf8(bytes).context("订阅解码后非 UTF-8")?
    };

    let mut nodes = Vec::new();
    for line in decoded.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with("vless://") {
            continue;
        }
        match parse_vless(line) {
            Ok(n) => nodes.push(n),
            Err(e) => tracing::warn!("跳过无法解析的节点: {e}"),
        }
    }
    if nodes.is_empty() {
        return Err(anyhow!("订阅中未解析到任何节点"));
    }
    Ok(nodes)
}

/// 解析单条 vless:// URI。
fn parse_vless(uri: &str) -> Result<VlessNode> {
    let u = Url::parse(uri).context("VLESS URI 解析失败")?;
    if u.scheme() != "vless" {
        return Err(anyhow!("非 vless 协议: {}", u.scheme()));
    }
    let uuid = u.username().to_string();
    if uuid.is_empty() {
        return Err(anyhow!("缺少 uuid"));
    }
    let address = u
        .host_str()
        .ok_or_else(|| anyhow!("缺少地址"))?
        .to_string();
    let port = u.port().ok_or_else(|| anyhow!("缺少端口"))?;

    let mut sni = address.clone();
    let mut host = address.clone();
    let mut path = "/".to_string();
    let mut fp = None;
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "sni" => sni = v.into_owned(),
            "host" => host = v.into_owned(),
            "path" => path = v.into_owned(),
            "fp" => fp = Some(v.into_owned()),
            _ => {}
        }
    }

    let name = u
        .fragment()
        .map(|f| percent_decode_str(f).decode_utf8_lossy().into_owned())
        .unwrap_or_default();

    Ok(VlessNode {
        uuid,
        address,
        port,
        sni,
        host,
        path,
        fp,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析明文_vless_列表() {
        // 用一个简化的合法 vless URI
        let body = "vless://11111111-2222-3333-4444-555555555555@1.2.3.4:443?security=tls&type=ws&host=a.com&sni=a.com&path=%2F&encryption=none#node1";
        let nodes = parse_subscription(body).unwrap();
        assert_eq!(nodes.len(), 1);
        let n = &nodes[0];
        assert_eq!(n.uuid, "11111111-2222-3333-4444-555555555555");
        assert_eq!(n.address, "1.2.3.4");
        assert_eq!(n.port, 443);
        assert_eq!(n.host, "a.com");
        assert_eq!(n.sni, "a.com");
        assert_eq!(n.path, "/");
        assert_eq!(n.name, "node1");
    }

    #[test]
    fn 解析_base64_编码订阅() {
        // 两行明文再 base64
        let plain = "vless://aaa@1.1.1.1:443?host=x.com&sni=x.com&path=%2F#A\nvless://bbb@2.2.2.2:8443?host=y.com&sni=y.com&path=%2F#B";
        let encoded = STANDARD.encode(plain);
        let nodes = parse_subscription(&encoded).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].address, "1.1.1.1");
        assert_eq!(nodes[1].port, 8443);
    }

    #[test]
    fn 空_或_无节点_返回错误() {
        assert!(parse_subscription("").is_err());
        assert!(parse_subscription("not a sub").is_err());
    }

    #[test]
    fn nodes_signature_集合相同顺序不同则一致() {
        let body = "vless://aaa@1.1.1.1:443?host=x.com&sni=x.com&path=%2F#A\nvless://bbb@2.2.2.2:8443?host=y.com&sni=y.com&path=%2F#B";
        let nodes = parse_subscription(body).unwrap();
        assert_eq!(nodes.len(), 2);
        let reversed: Vec<VlessNode> = nodes.iter().rev().cloned().collect();
        // 顺序不同但集合相同 -> 签名必须一致（防抖关键）
        assert_eq!(nodes_signature(&nodes), nodes_signature(&reversed));
        // 内容不同 -> 签名不同
        let only_first = vec![nodes[0].clone()];
        assert_ne!(nodes_signature(&nodes), nodes_signature(&only_first));
    }
}
