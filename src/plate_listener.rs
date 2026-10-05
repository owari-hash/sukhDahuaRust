//! Dahua camera plate detection via HTTP stream
//! Mirrors C# dugaarYavuulakh() — connects to snapManager.cgi and reads plate events

use std::collections::HashMap;
use std::time::Instant;
use std::sync::Mutex;
use log::warn;
use once_cell::sync::Lazy;
use tokio::sync::mpsc;
use digest_auth;

use crate::camera_manager::PlateEvent;

// Debounce: same plate from same camera within 3 seconds = skip
static LAST_PLATE: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Start plate listener for one camera — runs forever, reconnects on error.
/// `preferred_port` is re-probed (443 vs 80) on every (re)connect via `crate::probe`,
/// since the camera's open port can flip between attempts.
pub async fn run_plate_listener(ip: String, password: String, preferred_port: u16, plate_tx: mpsc::Sender<PlateEvent>) {
    let mut consecutive_failures: u32 = 0;

    loop {
        if consecutive_failures > 0 {
            let delay_secs = std::cmp::min(3 * 2u64.pow(consecutive_failures.saturating_sub(1).min(4)), 60);
            println!("[{ip}] Дахин холбогдохоор {delay_secs}s хүлээж байна (оролдлого #{consecutive_failures})");
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
        }

        let port = crate::probe::resolve_port(&ip, preferred_port).await;
        let (scheme, host) = crate::probe::scheme_and_host(&ip, port);
        let url = format!(
            "{scheme}://{host}/cgi-bin/snapManager.cgi?action=attachFileProc&Flags[0]=Event&Events=[TrafficJunction]&heartbeat=5"
        );

        println!("[{ip}] Plate listener эхлэж байна... ({url})");

        match connect_and_read(&ip, &password, &url, &plate_tx, &mut consecutive_failures).await {
            Ok(_) => {
                println!("[{ip}] Stream дууссан, дахин холбогдоно...");
            }
            Err(e) => {
                warn!("[{ip}] Холболтын алдаа (#{consecutive_failures}): {e}");
                crate::probe::invalidate(&ip);
            }
        }
        consecutive_failures += 1;
    }
}

async fn connect_and_read(
    ip: &str,
    password: &str,
    url: &str,
    plate_tx: &mpsc::Sender<PlateEvent>,
    consecutive_failures: &mut u32,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
    .danger_accept_invalid_certs(true)
    .build()?;

    // First request — get WWW-Authenticate header
    let first = client.get(url).send().await?;

    let resp = if first.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header = first.headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let mut prompt = digest_auth::parse(&auth_header)?;
        let context = digest_auth::AuthContext::new("admin", password, url);
        let answer = prompt.respond(&context)?.to_header_string();

        client
            .get(url)
            .header("Authorization", answer)
            .send()
            .await?
    } else {
        first
    };

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }

    println!("[{ip}] Амжилттай холбогдлоо — plate event хүлээж байна...");
    *consecutive_failures = 0;

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    use tokio_stream::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.starts_with("Events[0].Object.Text") {
                let plate = line
                    .replace("Events[0].Object.Text=", "")
                    .trim()
                    .trim_end_matches('\0')
                    .to_string();

                if plate.is_empty() { continue; }

                let key = format!("{ip}:{plate}");
                {
                    let now = Instant::now();
                    let mut map = LAST_PLATE.lock().unwrap();
                    if let Some(last) = map.get(&key) {
                        if now.duration_since(*last).as_secs() < 10 {
                            println!("[{ip}] Давхардсан plate давсан: {plate}");
                            continue;
                        }
                    }
                    map.insert(key, now);
                }

                println!("mashinii Dugaar burtgegdlee: {plate} | IP: {ip}");

                let event = PlateEvent { plate, camera_ip: ip.to_string() };
                if let Err(e) = plate_tx.try_send(event) {
                    warn!("[{ip}] Plate channel дүүрсэн: {e}");
                }
            }
        }
    }

    Ok(())
}
