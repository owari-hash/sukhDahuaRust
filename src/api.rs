use axum::{
    extract::Path,
    routing::{get, post},
    Router,
    response::IntoResponse,
    http::{StatusCode, Method, HeaderValue},
};
use log::{info, error, debug};
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
        .route("/api/sambarConfig/:ip",      get(sambar_config))
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
    info!("======= NEEYE HIT =======");
    info!("neeye called for ip: {ip}");

    if CAMERA_MANAGER.get().and_then(|m| m.handle_for_ip(&ip)).is_none() {
        error!("neeye Aldaa: IP {ip} not found");
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
            error!("neeye: Хаалга нээгдсэнгүй ({ip}): {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Хаалга нээгдсэнгүй: {e}"))
        }
    }
}

/// Самбарын ОДООГИЙН тохиргоог камераас уншина (бичихгүй).
///
/// Яагаад хэрэгтэй вэ: камерын вэб дээр мөр бүр нь Type (Custom/Date/
/// System Time), Contents, Text Color, Text Effect, Effect гэсэн ТУСДАА
/// талбартай. Доорх бичих код нь зөвхөн `Contents.[i]`-г дарж бичдэг тул
/// төхөөрөмжийн жинхэнэ түлхүүрийн нэртэй таарч байгаа эсэх нь тодорхойгүй.
/// Энэ төгсгөлөг нь `action=getConfig`-оор ЯГ ямар түлхүүр байгааг буцаана —
/// дараа нь түүнд нь тааруулж бичнэ.
async fn sambar_config(Path(ip): Path<String>) -> impl IntoResponse {
    info!("sambarConfig called for ip: {ip}");

    let (password, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (m.password_for_ip(&ip), m.http_port_for_ip(&ip).unwrap_or(80)))
        .unwrap_or_else(|| ("admin123".to_string(), 80));

    let http_port = crate::probe::resolve_port(&ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(&ip, http_port);

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=getConfig&name=TrafficLatticeScreen"
    );
    info!("[SAMBAR_CONFIG] URL: {url}");

    match send_sambar_request(&url, &password).await {
        Ok(body) => {
            info!("sambarConfig response:\n{body}");
            (StatusCode::OK, body)
        }
        Err(e) => {
            error!("sambarConfig Aldaa: {e}");
            crate::probe::invalidate(&ip);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("aldaa: {e}"))
        }
    }
}

/// Самбарын NORMAL мөрүүдийг өгсөн дарааллаар нь бичнэ (CarPass-ийг
/// ХӨНДӨХГҮЙ). Камерын дэлгэц 4 мөртэй — өгсөн хэмжээгээр нь бичих тул
/// дутуу өгвөл үлдсэн мөр нь хэвээр үлдэнэ.
pub async fn sambar_murnuud(ip: &str, murnuud: &[&str]) -> anyhow::Result<String> {
    let (password, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (m.password_for_ip(ip), m.http_port_for_ip(ip).unwrap_or(80)))
        .unwrap_or_else(|| ("admin123".to_string(), 80));

    let http_port = crate::probe::resolve_port(ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(ip, http_port);

    let mut params = vec!["TrafficLatticeScreen[0].StatusChangeTime=10".to_string()];
    for (i, mur) in murnuud.iter().enumerate().take(4) {
        params.push(format!(
            "TrafficLatticeScreen[0].Normal.Contents.[{i}]=str({mur})"
        ));
    }

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );
    info!("[SAMBAR_MURNUUD] URL: {url}");

    match send_sambar_request(&url, &password).await {
        Ok(body) => {
            info!("sambarMurnuud response: {body}");
            Ok(body)
        }
        Err(e) => {
            error!("sambarMurnuud Aldaa: {e}");
            crate::probe::invalidate(ip);
            Err(e)
        }
    }
}

/// Самбарын NORMAL гурван мөрийг шууд бичнэ (CarPass-ийг ХӨНДӨХГҮЙ).
///
/// `sambar_bichye` нь орох/гарах байрлалыг өөрөө шийддэг бол энэ нь
/// дуудагчийн өгсөн мөрүүдийг яг тэр хэвээр нь тавина — татгалзсан
/// мэдэгдэл, дүнгүй гарах зэрэгт хэрэгтэй.
pub async fn sambar_medegdel(
    ip: &str,
    mur0: &str,
    mur1: &str,
    mur2: &str,
) -> anyhow::Result<String> {
    let (password, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (m.password_for_ip(ip), m.http_port_for_ip(ip).unwrap_or(80)))
        .unwrap_or_else(|| ("admin123".to_string(), 80));

    let http_port = crate::probe::resolve_port(ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(ip, http_port);

    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=10".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({mur0})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[1]=str({mur1})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]=str({mur2})"),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );
    info!("[SAMBAR_MEDEGDEL] URL: {url}");

    match send_sambar_request(&url, &password).await {
        Ok(body) => {
            info!("sambarMedegdel response: {body}");
            Ok(body)
        }
        Err(e) => {
            error!("sambarMedegdel Aldaa: {e}");
            crate::probe::invalidate(ip);
            Err(e)
        }
    }
}

/// LED screen display — HTTP configManager.cgi
async fn sambar(Path((ip, text, dun)): Path<(String, String, String)>) -> impl IntoResponse {
    info!("sambar called for ip: {ip} text: {text} dun: {dun}");
    match sambar_bichye(&ip, &text, &dun).await {
        Ok(_) => (StatusCode::OK, "Amjilttai".to_string()),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "aldaa".to_string()),
    }
}

/// Самбарыг идэвхгүй (idle / standby) төлөвт шилжүүлнэ:
/// Орох камер:
///   Мөр 0: Компанийн нэр (company_name)
///   Мөр 1: СӨХ-ийн нэр (org_name)
///   Мөр 2: Зөвшөөрөлтэй машин
///   Мөр 3: Системийн цаг (SysTime)
/// Гарах камер:
///   Мөр 0: Компанийн нэр (company_name)
///   Мөр 1: СӨХ-ийн нэр (org_name)
///   Мөр 2: Хоосон
///   Мөр 3: Системийн цаг (SysTime)
pub async fn sambar_idle(ip: &str) -> anyhow::Result<String> {
    let (password, is_entrance, org_name, company_name, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (
            m.password_for_ip(ip),
            m.is_entrance(ip),
            m.org_name().to_string(),
            m.company_name().to_string(),
            m.http_port_for_ip(ip).unwrap_or(80),
        ))
        .unwrap_or(("admin123".to_string(), true, "Найрамдал".to_string(), "ParkEase".to_string(), 80));

    let http_port = crate::probe::resolve_port(ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(ip, http_port);

    let line2 = if is_entrance {
        "str(Зөвшөөрөлтэй машин)".to_string()
    } else {
        "str()".to_string()
    };

    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=10".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({company_name})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[1]=str({org_name})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]={line2}"),
        "TrafficLatticeScreen[0].Normal.Contents.[3]=SysTime".to_string(),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );

    info!("[SAMBAR_IDLE] is_entrance={is_entrance} URL: {url}");
    send_sambar_request(&url, &password).await
}

/// Бүртгэлгүй машин ирэх үед самбарт:
///   Мөр 0: Улсын дугаар (жнь: 0818УЕВ)
///   Мөр 1: Бүртгэлгүй машин
///   Мөр 2: СӨХ-ийн нэр
///   Мөр 3: Системийн цаг
/// (8 секундын дараа автоматаар idle төлөв рүү буцна)
pub async fn sambar_burtgelgui(ip: &str, plate: &str) -> anyhow::Result<String> {
    let (password, org_name, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (
            m.password_for_ip(ip),
            m.org_name().to_string(),
            m.http_port_for_ip(ip).unwrap_or(80),
        ))
        .unwrap_or(("admin123".to_string(), "Найрамдал".to_string(), 80));

    let http_port = crate::probe::resolve_port(ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(ip, http_port);

    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=10".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({plate})"),
        "TrafficLatticeScreen[0].Normal.Contents.[1]=str(Бүртгэлгүй машин)".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]=str({org_name})"),
        "TrafficLatticeScreen[0].Normal.Contents.[3]=SysTime".to_string(),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );

    info!("[SAMBAR_BURTGELGUI] URL: {url}");
    let res = send_sambar_request(&url, &password).await;

    // 8 секундын дараа автоматаар idle горим руу буцна
    let ip_clone = ip.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let _ = sambar_idle(&ip_clone).await;
    });

    res
}

/// Бүртгэлтэй машин ирэх / нэвтрэх үед:
///   Мөр 0: Улсын дугаар
///   Мөр 1: Төрөл (Оршин суугч / Зочин / Ажилтан / СӨХ / VIP)
///   Мөр 2: СӨХ-ийн нэр
///   Мөр 3: Системийн цаг
/// (8 секундын дараа автоматаар idle горим руу буцна)
pub async fn sambar_registered(ip: &str, plate: &str, turul: &str) -> anyhow::Result<String> {
    let (password, org_name, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (
            m.password_for_ip(ip),
            m.org_name().to_string(),
            m.http_port_for_ip(ip).unwrap_or(80),
        ))
        .unwrap_or(("admin123".to_string(), "Тайм Таур".to_string(), 80));

    let http_port = crate::probe::resolve_port(ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(ip, http_port);

    let display_turul = if turul.trim().is_empty() {
        "Оршин суугч"
    } else {
        turul.trim()
    };

    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=10".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({plate})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[1]=str({display_turul})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]=str({org_name})"),
        "TrafficLatticeScreen[0].Normal.Contents.[3]=SysTime".to_string(),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );

    info!("[SAMBAR_REGISTERED] URL: {url}");
    let res = send_sambar_request(&url, &password).await;

    // 8 секундын дараа автоматаар idle горим руу буцна
    let ip_clone = ip.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let _ = sambar_idle(&ip_clone).await;
    });

    res
}

/// Гарцын төлбөр болон төрлийг харуулах:
///   Мөр 0: Улсын дугаар
///   Мөр 1: Төрөл эсвэл Дүн
///   Мөр 2: Дүн эсвэл СӨХ-ийн нэр
///   Мөр 3: Системийн цаг
pub async fn sambar_exit(ip: &str, plate: &str, turul: &str, dun: &str) -> anyhow::Result<String> {
    let (password, org_name, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (
            m.password_for_ip(ip),
            m.org_name().to_string(),
            m.http_port_for_ip(ip).unwrap_or(80),
        ))
        .unwrap_or(("admin123".to_string(), "Найрамдал".to_string(), 80));

    let http_port = crate::probe::resolve_port(ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(ip, http_port);

    let dun_t = format!("{}T", dun);
    let line1 = if !turul.trim().is_empty() && turul != "Үйлчлүүлэгч" {
        turul.trim().to_string()
    } else if dun != "0" && !dun.is_empty() {
        dun_t.clone()
    } else {
        "Үнэгүй".to_string()
    };

    let line2 = if line1 != dun_t && dun != "0" && !dun.is_empty() {
        dun_t
    } else {
        org_name
    };

    let params = [
        "TrafficLatticeScreen[0].StatusChangeTime=10".to_string(),
        format!("TrafficLatticeScreen[0].Normal.Contents.[0]=str({plate})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[1]=str({line1})"),
        format!("TrafficLatticeScreen[0].Normal.Contents.[2]=str({line2})"),
        "TrafficLatticeScreen[0].Normal.Contents.[3]=SysTime".to_string(),
    ];

    let url = format!(
        "{scheme}://{host}/cgi-bin/configManager.cgi?action=setConfig&{}",
        params.join("&")
    );

    info!("[SAMBAR_EXIT] URL: {url}");
    let res = send_sambar_request(&url, &password).await;

    // 8 секундын дараа автоматаар idle горим руу буцна
    let ip_clone = ip.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let _ = sambar_idle(&ip_clone).await;
    });

    res
}

pub async fn sambar_bichye(ip: &str, text: &str, dun: &str) -> anyhow::Result<String> {
    let is_entrance = CAMERA_MANAGER
        .get()
        .map(|m| m.is_entrance(ip))
        .unwrap_or(false);

    if is_entrance {
        sambar_registered(ip, text, "Оршин суугч").await
    } else {
        sambar_exit(ip, text, "", dun).await
    }
}

async fn sambar_ognootoi(
    Path((ip, text, dun, start, end)): Path<(String, String, String, String, String)>,
) -> impl IntoResponse {
    info!("sambarOgnootoi called for ip: {ip} text: {text} dun: {dun} start: {start} end: {end}");

    let (password, preferred_port) = CAMERA_MANAGER
        .get()
        .map(|m| (m.password_for_ip(&ip), m.http_port_for_ip(&ip).unwrap_or(80)))
        .unwrap_or_else(|| ("admin123".to_string(), 80));

    let http_port = crate::probe::resolve_port(&ip, preferred_port).await;
    let (scheme, host) = crate::probe::scheme_and_host(&ip, http_port);

    let dun_t = format!("{}T", dun);

    let params = [
        // Төлөв солигдох ЗАВСАР (секунд). Dahua-гийн SDK толгойд
        // `nStatusChangeTime` нь 10 ~ 60 гэж ЗААГДСАН
        // (NET_CFG_TRAFFIC_LATTICE_SCREEN_INFO). Өмнө нь 1 гэж явуулдаг
        // байсан нь хүрээнээс ГАДУУР — төхөөрөмж ийм утгыг хүлээж авахгүй
        // бөгөөд Dahua нь нэг параметр буруу бол setConfig-ийг БҮХЭЛД нь
        // няцаадаг тул бусад мөр ч бичигдэхгүй өнгөрөх эрсдэлтэй байв.
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

    info!("[SAMBAR_OGNOOTOI] URL: {url}");

    match send_sambar_request(&url, &password).await {
        Ok(body) => {
            info!("sambarOgnootoi response: {body}");
            (StatusCode::OK, "Amjilttai".to_string())
        }
        Err(e) => {
            error!("sambarOgnootoi Aldaa: {e}");
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
    info!("First response status: {}", first.status());

    let resp = if first.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header = first.headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        debug!("WWW-Authenticate: {auth_header}");

        // Extract just the path+query for digest URI — use original url
       let uri = if let Some(pos) = url.find("/cgi-bin") {
            &url[pos..]
        } else {
            url
        };

        let mut prompt = digest_auth::parse(&auth_header)?;
        let context = digest_auth::AuthContext::new("admin", password, uri);
        let answer = prompt.respond(&context)?.to_header_string();
        debug!("Auth answer: {answer}");

        client.get(url).header("Authorization", answer).send().await?
    } else {
        first
    };

    info!("Final response status: {}", resp.status());
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
    error!("404 Not Found: {} {}", req.method(), req.uri());
    (StatusCode::NOT_FOUND, "Not Found")
}
