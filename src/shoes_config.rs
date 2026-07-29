//! 生成 shoes 引擎的 YAML 配置。
//! 把订阅解析出的 VlessNode 列表 + 本地 Config，组装成 shoes 可加载的配置：
//!   - socks5 server（本地入口，可选账号密码认证）
//!   - rules: 全流量 allow，client_chains 内联所有节点做 round-robin 负载均衡
//!   - 每个节点 = tls(websocket(vless)) 嵌套协议栈，连 CF 优选 IP
//!
//! 安全：生成的 YAML 含节点 UUID / SNI 等订阅机密，必须只写到 tmpfs（0600），
//! 不得打入镜像、不得写入日志。

use anyhow::Result;
use serde_yaml::{Mapping, Value};

use crate::config::Config;
use crate::subscription::VlessNode;

/// 生成完整 shoes 配置（YAML 数组，含单个 server 项）。
pub fn build_shoes_config(cfg: &Config, nodes: &[VlessNode]) -> Value {
    let server = build_server(cfg, nodes);
    Value::Sequence(vec![Value::Mapping(server)])
}

/// 构造 socks5 server 项。
fn build_server(cfg: &Config, nodes: &[VlessNode]) -> Mapping {
    let mut server = Mapping::new();
    server.insert(
        Value::String("address".into()),
        Value::String(format!("{}:{}", cfg.addr, cfg.port)),
    );

    // protocol: socks（可选认证）
    let mut protocol = Mapping::new();
    protocol.insert(Value::String("type".into()), Value::String("socks".into()));
    if cfg.has_auth() {
        protocol.insert(
            Value::String("username".into()),
            Value::String(cfg.user.clone().unwrap_or_default()),
        );
        protocol.insert(
            Value::String("password".into()),
            Value::String(cfg.pass.clone().unwrap_or_default()),
        );
    }
    server.insert(Value::String("protocol".into()), Value::Mapping(protocol));

    // rules: 全流量走代理，client_chains 内联所有节点（round-robin）
    let mut rule = Mapping::new();
    rule.insert(Value::String("masks".into()), Value::String("0.0.0.0/0".into()));
    rule.insert(Value::String("action".into()), Value::String("allow".into()));
    rule.insert(
        Value::String("client_chains".into()),
        Value::Sequence(nodes.iter().map(|n| Value::Mapping(build_node_client(n))).collect()),
    );
    server.insert(Value::String("rules".into()), Value::Sequence(vec![Value::Mapping(rule)]));

    server
}

/// 构造单个节点的 client config：tls(websocket(vless(user_id)))。
fn build_node_client(n: &VlessNode) -> Mapping {
    // 最内层 VLESS
    let mut vless = Mapping::new();
    vless.insert(Value::String("type".into()), Value::String("vless".into()));
    vless.insert(Value::String("user_id".into()), Value::String(n.uuid.clone()));

    // WebSocket 层
    let mut ws = Mapping::new();
    ws.insert(Value::String("type".into()), Value::String("websocket".into()));
    ws.insert(Value::String("matching_path".into()), Value::String(n.path.clone()));
    // WS Host 头必须用节点域名（CF worker 按域名路由），不能用优选 IP
    let mut headers = Mapping::new();
    headers.insert(Value::String("Host".into()), Value::String(n.host.clone()));
    ws.insert(Value::String("matching_headers".into()), Value::Mapping(headers));
    ws.insert(Value::String("protocol".into()), Value::Mapping(vless));

    // TLS 层（SNI 用节点域名，不用 IP）
    let mut tls = Mapping::new();
    tls.insert(Value::String("type".into()), Value::String("tls".into()));
    tls.insert(Value::String("sni_hostname".into()), Value::String(n.sni.clone()));
    tls.insert(Value::String("protocol".into()), Value::Mapping(ws));

    // client config 顶层
    let mut client = Mapping::new();
    client.insert(
        Value::String("address".into()),
        Value::String(format!("{}:{}", n.address, n.port)),
    );
    client.insert(Value::String("protocol".into()), Value::Mapping(tls));
    client
}

/// 序列化为 YAML 字符串。
pub fn to_yaml(cfg: &Config, nodes: &[VlessNode]) -> Result<String> {
    let v = build_shoes_config(cfg, nodes);
    Ok(serde_yaml::to_string(&v)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_cfg(auth: bool) -> Config {
        Config {
            sub_url: String::new(),
            proxy_type: "socks5".to_string(),
            addr: "0.0.0.0".to_string(),
            port: 1080,
            user: if auth { Some("u".to_string()) } else { None },
            pass: if auth { Some("p".to_string()) } else { None },
        }
    }

    fn mk_node(addr: &str, uuid: &str) -> VlessNode {
        VlessNode {
            uuid: uuid.to_string(),
            address: addr.to_string(),
            port: 443,
            sni: "fq.ferocks.com".to_string(),
            host: "fq.ferocks.com".to_string(),
            path: "/".to_string(),
            fp: None,
            name: format!("node-{addr}"),
        }
    }

    #[test]
    fn yaml_含_socks5_入口与认证() {
        let yaml = to_yaml(&mk_cfg(true), &[mk_node("1.2.3.4", "uuid-1")]).unwrap();
        assert!(yaml.contains("type: socks"));
        assert!(yaml.contains("username: u"));
        assert!(yaml.contains("password: p"));
        assert!(yaml.contains("address: '0.0.0.0:1080'") || yaml.contains("address: 0.0.0.0:1080"));
    }

    #[test]
    fn yaml_无认证时不输出账密() {
        let yaml = to_yaml(&mk_cfg(false), &[mk_node("1.2.3.4", "uuid-1")]).unwrap();
        assert!(yaml.contains("type: socks"));
        assert!(!yaml.contains("username"));
        assert!(!yaml.contains("password"));
    }

    #[test]
    fn yaml_每个节点生成_vless_ws_tls_嵌套() {
        let nodes = vec![mk_node("1.1.1.1", "uuid-a"), mk_node("2.2.2.2", "uuid-b")];
        let yaml = to_yaml(&mk_cfg(false), &nodes).unwrap();
        // 两个节点
        assert_eq!(yaml.matches("type: vless").count(), 2);
        assert_eq!(yaml.matches("type: websocket").count(), 2);
        assert_eq!(yaml.matches("type: tls").count(), 2);
        // uuid 出现
        assert!(yaml.contains("uuid-a"));
        assert!(yaml.contains("uuid-b"));
        // SNI 域名
        assert!(yaml.contains("sni_hostname: fq.ferocks.com"));
        // 优选 IP 地址
        assert!(yaml.contains("1.1.1.1:443"));
    }

    #[test]
    fn yaml_可被_shoes_风格的嵌套解析回读() {
        let yaml = to_yaml(&mk_cfg(false), &[mk_node("9.9.9.9", "uuid-x")]).unwrap();
        // 反序列化确保是合法 YAML 且顶层是数组
        let parsed: Value = serde_yaml::from_str(&yaml).unwrap();
        assert!(parsed.as_sequence().is_some());
        let server = parsed.as_sequence().unwrap()[0].as_mapping().unwrap();
        assert_eq!(server.get(Value::String("address".into())).unwrap(), "0.0.0.0:1080");
    }
}
