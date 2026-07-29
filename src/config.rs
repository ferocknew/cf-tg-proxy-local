//! 配置模块：从环境变量加载运行配置，机密字段在 Debug 输出时脱敏。

use std::env;
use std::fmt;

/// 运行配置。机密字段在 Debug 输出时脱敏，绝不泄露明文。
#[derive(Clone)]
pub struct Config {
    /// edgetunnel 订阅地址（含 token，机密）
    pub sub_url: String,
    /// 代理类型：socks5 / mtproto（当前实现 socks5）
    pub proxy_type: String,
    /// 本地监听地址
    pub addr: String,
    /// 本地监听端口
    pub port: u16,
    /// socks5 用户名（为空表示不启用认证）
    pub user: Option<String>,
    /// socks5 密码（机密）
    pub pass: Option<String>,
}

impl Config {
    /// 从环境变量加载（可由 dotenvy 注入 .env）。
    pub fn from_env() -> Self {
        let user = env::var("TG_PROXY_USER").ok().filter(|s| !s.is_empty());
        let pass = env::var("TG_PROXY_PASS").ok().filter(|s| !s.is_empty());

        Self {
            sub_url: env::var("SUB_URL").unwrap_or_default(),
            proxy_type: env::var("TG_PROXY_TYPE").unwrap_or_else(|_| "socks5".to_string()),
            addr: env::var("TG_PROXY_ADDR").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("TG_PROXY_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1080),
            user,
            pass,
        }
    }

    /// 是否启用 socks5 用户名/密码认证（RFC 1929）。
    pub fn has_auth(&self) -> bool {
        self.user.is_some() && self.pass.is_some()
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("sub_url", &"<redacted>")
            .field("proxy_type", &self.proxy_type)
            .field("addr", &self.addr)
            .field("port", &self.port)
            .field("has_auth", &self.has_auth())
            .field("pass", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_不泄露密码与订阅地址() {
        let cfg = Config {
            sub_url: "https://secret.example.com/sub?token=abc".to_string(),
            proxy_type: "socks5".to_string(),
            addr: "0.0.0.0".to_string(),
            port: 1080,
            user: Some("u".to_string()),
            pass: Some("super-secret-password".to_string()),
        };
        let s = format!("{cfg:?}");
        assert!(s.contains("<redacted>"));
        assert!(!s.contains("super-secret-password"));
        assert!(!s.contains("secret.example.com"));
    }

    #[test]
    fn has_auth_仅当账号密码都非空() {
        let mk = |u: Option<&str>, p: Option<&str>| Config {
            sub_url: String::new(),
            proxy_type: "socks5".to_string(),
            addr: "0.0.0.0".to_string(),
            port: 1080,
            user: u.map(|s| s.to_string()),
            pass: p.map(|s| s.to_string()),
        };
        assert!(mk(Some("u"), Some("p")).has_auth());
        assert!(!mk(Some("u"), None).has_auth());
        assert!(!mk(None, Some("p")).has_auth());
        assert!(!mk(None, None).has_auth());
    }
}
