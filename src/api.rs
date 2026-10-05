use axum::{
    extract::Path,
    routing::{get, post},
    Router,
    response::IntoResponse,
    http::{StatusCode, Method, HeaderValue},
};
use log::info;
use tokio::net::TcpListener;
use tower_http::cors::{CorsLayer, Any};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::camera_manager::CAMERA_MANAGER;

// Scheme unknown for the bare-IP origin (no TLS cert on an IP is common), so
// both are allowed rather than guessing wrong and silently locking it out.
const ALLOWED_ORIGINS: [&str; 3] = [
    "https://amarhome.mn",
    "http://103.236.194.99",
    "https://103.236.194.99",
];

pub async fn run_api_server(port: u16) {
    let origins: Vec<HeaderValue> = ALLOWED_ORIGINS
        .iter()
        .map(|o| o.parse().expect("valid origin"))
        .collect();

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_origin(origins)
        .allow_headers(Any)
        .allow_private_network(true)
        .max_age(std::time::Duration::from_secs(3600));

    let app = Router::new()
        .route("/api/neeye/:ip",             get(neeye))
        .route("/api/sambar/:ip/:text/:dun", get(sambar))
        .route("/api/sambarOgnootoi/:ip/:text/:dun/:start/:end",    get(sambar_ognootoi))
        .route("/api/restartConnections",    post(restart_connections))
        .route("/api/health",                get(health))
        .fallback(handler_404)
        .layer(cors);

    let addr = format!("0.0.0.0:{port}");
    info!("API server listening on {addr}");

    let listener = TcpListener::bind(&addr).await.expect("Cannot bind API port");
    axum::serve(listener, app).await.expect("API server crashed");
}

/// Open barrier gate — called by frontend after server approves plate
async fn neeye(Path(ip): Path<String>) -> impl IntoResponse {
    println!("======= NEEYE HIT =======");
    println!("neeye called for ip: {ip}");

    if CAMERA_MANAGER.get().and_then(|m| m.handle_for_ip(&ip)).is_none() {
        println!("neeye Aldaa: IP {ip} not found");
        return (StatusCode::INTERNAL_SERVER_ERROR, "aldaa".to_string());
    }

    // open_gate calls blocking SDK FFI — must use spawn_blocking
    let ip_clone = ip.clone();
    let result = tokio::task::spawn_blocking(move || {
        CAMERA_MANAGER.get()
            .map(|m| m.open_gate(&ip_clone, None))
            .unwrap_or_else(|| Err("camera_manager_not_initialized".to_string()))
    }).await.unwrap_or_else(|_| Err("task_join_error".to_string()));

    match result {
        Ok(()) => (StatusCode::OK, "Amjilttai".to_string()),
        Err(e) => {
            println!("neeye: Хаалга нээгдсэнгүй ({ip}): {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Хаалга нээгдсэнгүй: {e}"))
        }
    }
}

/// LED screen display — HTTP configManager.cgi
async fn sambar(Path((ip, text, dun)): Path<(String, String, String)>) -> impl IntoResponse {
    println!("sambar called for ip: {ip} text: {text} dun: {dun}");

    let (password, is_entrance, org_name, company_name, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (
            m.password_for_ip(&ip),
            m.is_entrance(&ip),
            m.org_name().to_string(),
            m.company_name().to_string(),
            m.http_port_for_ip(&ip).unwrap_or(80),
        ))
        .unwrap_or(("admin123".to_string(), false, "Найрамдал".to_string(), "Найрамдал".to_string(), 80));

    let http_port = crate::probe::resolve_port(&ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(&ip, http_port);

    let dun_t = format!("{}T", dun);
    let line1 = if is_entrance { org_name.clone() } else { dun_t.clone() };
    let line2 = if is_entrance { "Зөврөөшөлтэй машин".to_string() } else { org_name.clone() };
    let carpass_0 = if is_entrance { company_name.clone() } else { "".to_string() };
    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=1".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({text})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[1]=str({line1})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]=str({line2})"),
        format!("TrafficLatticeScreen[0].CarPass.Contents.[0]=str({carpass_0})"),
        "TrafficLatticeScreen[0].CarPass.Contents.[1]=SysTime".to_string(),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );

    println!("[SAMBAR] URL: {url}");

    match send_sambar_request(&url, &password).await {
        Ok(body) => {
            println!("sambar response: {body}");
            (StatusCode::OK, "Amjilttai".to_string())
        }
        Err(e) => {
            println!("sambar Aldaa: {e}");
            crate::probe::invalidate(&ip);
            (StatusCode::INTERNAL_SERVER_ERROR, "aldaa".to_string())
        }
    }
}

async fn sambar_ognootoi(
    Path((ip, text, dun, start, end)): Path<(String, String, String, String, String)>,
) -> impl IntoResponse {
    println!("sambarOgnootoi called for ip: {ip} text: {text} dun: {dun} start: {start} end: {end}");

    let (password, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (m.password_for_ip(&ip), m.http_port_for_ip(&ip).unwrap_or(80)))
        .unwrap_or_else(|| ("admin123".to_string(), 80));

    let http_port = crate::probe::resolve_port(&ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(&ip, http_port);

    let dun_t = format!("{}T", dun);

    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=1".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({text})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[1]=str({dun_t})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]=str({start})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[3]=str({end})"),
        format!("TrafficLatticeScreen[0].CarPass.Contents.[0]=str({text})"),
        format!("TrafficLatticeScreen[0].CarPass.Contents.[1]=str({dun_t})"),
        "TrafficLatticeScreen[0].CarPass.Contents.[2]=str()".to_string(),
        "TrafficLatticeScreen[0].CarPass.Contents.[3]=SysTime".to_string(),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );

    println!("[SAMBAR_OGNOOTOI] URL: {url}");

    match send_sambar_request(&url, &password).await {
        Ok(body) => {
            println!("sambarOgnootoi response: {body}");
            (StatusCode::OK, "Amjilttai".to_string())
        }
        Err(e) => {
            println!("sambarOgnootoi Aldaa: {e}");
            crate::probe::invalidate(&ip);
            (StatusCode::INTERNAL_SERVER_ERROR, "aldaa".to_string())
        }
    }
}
async fn send_sambar_request(url: &str, password: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(5))
    .danger_accept_invalid_certs(true)  // ← нэм
    .build()?;

    let first = client.get(url).send().await?;
    println!("First response status: {}", first.status());

    let resp = if first.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header = first.headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        println!("WWW-Authenticate: {auth_header}");

        // Extract just the path+query for digest URI — use original url
       let uri = if let Some(pos) = url.find("/cgi-bin") {
            &url[pos..]
        } else {
            url
        };

        let mut prompt = digest_auth::parse(&auth_header)?;
        let context = digest_auth::AuthContext::new("admin", password, uri);
        let answer = prompt.respond(&context)?.to_header_string();
        println!("Auth answer: {answer}");

        client.get(url).header("Authorization", answer).send().await?
    } else {
        first
    };

    println!("Final response status: {}", resp.status());
    let body = resp.text().await.unwrap_or_default();
    Ok(body)
}

async fn restart_connections() -> impl IntoResponse {
    info!("Manual connection restart requested via API");
    tokio::task::spawn_blocking(|| {
        if let Some(mgr) = CAMERA_MANAGER.get() {
            mgr.reconnect_all();
        }
    });
    let count = CAMERA_MANAGER.get().map(|m| m.camera_count()).unwrap_or(0);
    (StatusCode::OK, format!("Restart initiated. {count} camera(s) in list"))
}

async fn health() -> impl IntoResponse {
    let count = CAMERA_MANAGER.get().map(|m| m.camera_count()).unwrap_or(0);
    (StatusCode::OK, format!("OK | cameras={count}"))
}

async fn handler_404(req: axum::extract::Request) -> impl IntoResponse {
    println!("404 Not Found: {} {}", req.method(), req.uri());
    (StatusCode::NOT_FOUND, "Not Found")
}
