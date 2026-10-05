//! Dahua камерууд зарим үед HTTPS(443)/HTTP(80) портын аль нэгийг л нээлттэй байлгадаг
//! бөгөөд энэ нь өөрөө өдрөөс өдөрт солигдож болдог тул config.toml дахь статик
//! http_port рүү үргэлж найдаж болохгүй. Энэ модуль аль нь одоо ажиллаж байгааг
//! TCP-ээр шалгаж, кэшлээд өгнө — sambar болон plate listener хоёулаа үүнийг ашиглана.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use once_cell::sync::Lazy;
use tokio::net::TcpStream;
use tokio::time::timeout;

struct CacheEntry {
    port:       u16,
    checked_at: Instant,
}

static CACHE: Lazy<Mutex<HashMap<String, CacheEntry>>> = Lazy::new(|| Mutex::new(HashMap::new()));
const CACHE_TTL:      Duration = Duration::from_secs(60);
const PROBE_TIMEOUT:  Duration = Duration::from_millis(800);

/// `ip` дээр одоо ажиллаж байгаа портыг (443 эсвэл 80) буцаана.
/// `preferred` (config.toml-с) хоёулаа нээлттэй үед давуу эрхтэй байна.
pub async fn resolve_port(ip: &str, preferred: u16) -> u16 {
    if let Some(entry) = CACHE.lock().unwrap().get(ip) {
        if entry.checked_at.elapsed() < CACHE_TTL {
            return entry.port;
        }
    }

    let port = probe(ip, preferred).await;
    CACHE.lock().unwrap().insert(ip.to_string(), CacheEntry { port, checked_at: Instant::now() });
    port
}

/// Тухайн IP-н кэшийг хүчингүй болгоно — жинхэнэ хүсэлт холболтын алдаагаар
/// бүтэлгүйтвэл дуудаж, дараагийн оролдлого дахин шалгадаг болгоно (порт өөрчлөгдсөн ч мэдэрнэ).
pub fn invalidate(ip: &str) {
    CACHE.lock().unwrap().remove(ip);
}

async fn probe(ip: &str, preferred: u16) -> u16 {
    let other = if preferred == 443 { 80 } else { 443 };
    if tcp_open(ip, preferred).await {
        return preferred;
    }
    if tcp_open(ip, other).await {
        println!("[{ip}] Port {preferred} хаалттай байна, {other} дээр ажиллаж байна — автоматаар сэлгэлээ");
        return other;
    }
    // хоёул хаалттай бол configured-оо буцаагаад дуудагч тал алдааг харуулна
    preferred
}

async fn tcp_open(ip: &str, port: u16) -> bool {
    matches!(timeout(PROBE_TIMEOUT, TcpStream::connect((ip, port))).await, Ok(Ok(_)))
}

/// `443` бол https, эсвэл http — стандарт бус портод IP:PORT хэлбэрээр URL host бичнэ.
pub fn scheme_and_host(ip: &str, port: u16) -> (&'static str, String) {
    let scheme = if port == 443 { "https" } else { "http" };
    let host = if (scheme == "http" && port == 80) || (scheme == "https" && port == 443) {
        ip.to_string()
    } else {
        format!("{ip}:{port}")
    };
    (scheme, host)
}