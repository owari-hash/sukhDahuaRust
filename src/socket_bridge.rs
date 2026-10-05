use rust_socketio::{ClientBuilder, Payload, RawClient, TransportType};
use log::{info, error, warn, debug};
use crate::camera_manager::CAMERA_MANAGER;
use crate::config::BarilgaTarget;
use once_cell::sync::Lazy;

static HTTP_CLIENT: Lazy<reqwest::blocking::Client> = Lazy::new(|| {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .tcp_nodelay(true)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(10)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
});

/// SDK-гаар нэвтрдэг Dahua хаалганы IP-ууд (`[[cameras]]`).
///
/// `rewrite_rtsp_url` зөвхөн эдний хаягыг Dahua хэлбэрт болгоно.
/// Барилгад бодит Hikvision NVR байж болно — тэргэнийг хөрвүүлвэл
/// r2w зөв замыг олохгүй болно.
///
/// Утга нь тухайн камерын RTSP ЗАМ (`[[cameras]].root`). Бүх Dahua марк
/// `cam/realmonitor` -аар үйлчилдэггүй тул зөв замыг тохиргооноос авна.
static DAHUA_KAMERUUD: Lazy<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// `main` энээг хөтөчийн хүсэлт ирэхээс ӨМНӨ дуудна.
pub fn dahua_ipuud_tavya<I: IntoIterator<Item = (String, String)>>(kameruud: I) {
    if let Ok(mut s) = DAHUA_KAMERUUD.lock() {
        s.clear();
        for (ip, zam) in kameruud {
            let ip = ip.trim().to_string();
            if !ip.is_empty() {
                s.insert(ip, zam);
            }
        }
        debug!("BRIDGE | Dahua камер: {} бүртгэв", s.len());
    }
}

/// Тухайн хаяг танигдсан Dahua төхөөрөмж рүү чиглэсэн бол RTSP замыг нь.
fn dahua_zam(ip: &str) -> Option<String> {
    DAHUA_KAMERUUD
        .lock()
        .ok()
        .and_then(|s| s.get(ip.trim()).cloned())
}

/// Socket.io Bridge for a specific building target
pub fn start_socket_bridge(vps_url: String, target: BarilgaTarget) {
    std::thread::spawn(move || {
        let tag = format!("BRIDGE {}", target.label);
        loop {
            info!("🔌 [{tag}] Connecting to VPS: {vps_url}");
            let bid_connect  = target.barilgiin_id.clone();
            let bid_fallback = target.barilgiin_id.clone();
            let tag_connect  = tag.clone();
            let tag_offer    = tag.clone();
            let tag_open     = tag.clone();
            let tag_disc     = tag.clone();
            let tag_err      = tag.clone();
            let tag_urg_on   = tag.clone();
            let tag_samb     = tag.clone();
            let tag_urg_off  = tag.clone();

            let socket_res = ClientBuilder::new(&vps_url)
                .namespace("/")
                .transport_type(TransportType::Polling)
                .on("connect", move |_payload, socket: RawClient| {
                    info!("✅ [{tag_connect}] Connected. Registering building: {bid_connect}");
                    match socket.emit("register-gate-worker", bid_connect.clone()) {
                        Ok(_)  => info!("📤 [{tag_connect}] register-gate-worker sent OK"),
                        Err(e) => error!("❌ [{tag_connect}] register-gate-worker failed: {e:?}"),
                    }
                })
                .on("open", |_payload, _| {
                    info!("✅ [BRIDGE] Event: 'open'. Connection is active.");
                })
                .on("close", move |_, _| {
                    warn!("❌ [{tag_disc}] Event: 'close'. Connection lost.");
                })
                .on("error", move |err, _| {
                    error!("❌ [{tag_err}] Socket error: {err:#?}");
                })
                .on("sambar-show", move |payload: Payload, _socket: RawClient| {
                    handle_sambar_show(payload, &tag_samb);
                })
                .on("execute-open", move |payload: Payload, socket: RawClient| {
                    handle_execute_open(payload, socket, &tag_open);
                })
                .on("webrtc-offer", move |payload: Payload, socket: RawClient| {
                    handle_webrtc_offer(payload, socket, &tag_offer);
                })
                // MediaMTX дээр уншигч гарч ирэхэд backend эдгээрийг илгээнэ.
                .on("urgats-start", move |payload: Payload, _socket: RawClient| {
                    handle_urgats(payload, &tag_urg_on, true);
                })
                .on("urgats-stop", move |payload: Payload, _socket: RawClient| {
                    handle_urgats(payload, &tag_urg_off, false);
                })
                .connect();

            match socket_res {
                Ok(socket) => {
                    info!("🚀 [{tag}] Socket connected successfully. Starting heartbeat loop.");
                    // Fallback registration in case 'connect' event missed
                    let _ = socket.emit("register-gate-worker", bid_fallback.clone());

                    let mut tick = 0u32;
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(15));
                        tick += 1;

                        if let Err(e) = socket.emit("ping", "") {
                            error!("🔄 [{tag}] Connection lost (emit ping failed): {e:?}");
                            break;
                        }

                        // Re-register every 4 ticks (~60s) so VPS never loses this worker
                        if tick % 4 == 0 {
                            if let Err(e) = socket.emit("register-gate-worker", bid_fallback.clone()) {
                                error!("❌ [{tag}] Periodic re-register failed: {e:?}");
                            } else {
                                debug!("📤 [{tag}] Periodic re-register OK (tick {tick})");
                            }
                        }
                    }

                    let _ = socket.disconnect();
                }
                Err(e) => {
                    error!("❌ [{tag}] Connection failed to start: {e:?}");
                }
            }

            info!("🔄 [{tag}] Reconnecting in 5 seconds...");
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

fn handle_execute_open(payload: Payload, socket: RawClient, tag: &str) {
    let json_str = match payload {
        Payload::String(s) => s,
        Payload::Binary(b) => String::from_utf8_lossy(&b).to_string(),
    };

    let v: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            error!("❌ [{tag} GATE] Failed to parse Execute Open JSON: {json_str} ({e})");
            return;
        }
    };

    let ip = v.get("ip").and_then(|i| i.as_str()).unwrap_or_default().to_string();
    let plate = v.get("plate").and_then(|p| p.as_str()).unwrap_or_default().to_string();
    let command_id = v.get("commandId").and_then(|c| c.as_str()).unwrap_or_default().to_string();
    let plate_log = if plate.is_empty() { "-" } else { &plate };

    if ip.is_empty() {
        error!("❌ [{tag} GATE] Execute Open payload missing 'ip' field (plate={plate_log})");
        return;
    }

    info!("🚪 [{tag} GATE] Received: ip={ip} plate={plate_log} id={command_id}");

    let result = CAMERA_MANAGER.get()
        .map(|m| m.open_gate(&ip, Some(plate.as_str())))
        .unwrap_or_else(|| Err("camera_manager_not_initialized".to_string()));

    match &result {
        Ok(()) => info!("🚪 [{tag} GATE] Result: ip={ip} plate={plate_log} success=true"),
        Err(e) => error!("🚪 [{tag} GATE] Result: ip={ip} plate={plate_log} success=false error={e}"),
    }

    // ЗӨВХӨН ОРЦЫН самбарт бичнэ.
    //
    // `execute-open` нь гарах үед ч ирдэг. Гарцын самбарыг `sambar-show`
    // (→ `sambar_exit`) аль хэдийн бичдэг бөгөөд тэнд төрөл нь хадгалагдсан
    // `uilchluulegch.turul`-ээс, ДҮН нь хамт ирдэг — найдвартай. Харин
    // `execute-open`-ий төрөл нь урилгын хайлтаас гардаг бөгөөд гарах үед
    // урилга нь идэвхтэй (tuluv 0/1) байхаа больсон байж болно. Хоёулаа
    // нэг самбар руу бичвэл уралдаж, сүүлд нь буусан нь ялна — зочин
    // "Үйлчлүүлэгч" болж харагдах эрсдэлтэй. Иймд гарцад бичихгүй.
    let orts = crate::camera_manager::CAMERA_MANAGER
        .get()
        .map(|m| m.is_entrance(&ip))
        .unwrap_or(true);

    if result.is_ok() && !plate.is_empty() && orts {
        // Сервер төрөл илгээгээгүй бол ОРШИН СУУГЧ гэж ТААХГҮЙ — тэр нь
        // төлбөртэй үйлчлүүлэгчийг үнэгүй оршин суугч мэт харуулдаг байв.
        // Хоосон үлдээвэл доорх бичигч нь СӨХ-ийн нэрийг тавина.
        let turul = v.get("turul").and_then(|t| t.as_str()).unwrap_or_default().to_string();
        let ip_s = ip.clone();
        let plate_s = plate.clone();
        let turul_s = turul.clone();
        let tag_s = tag.to_string();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("❌ [{tag_s} SAMBAR] runtime үүсгэж чадсангүй: {e}");
                    return;
                }
            };
            let khariu = rt.block_on(crate::api::sambar_registered(&ip_s, &plate_s, &turul_s));
            match khariu {
                Ok(body) => info!(
                    "🔆 [{tag_s} SAMBAR] ip={ip_s} plate={plate_s} turul={turul_s} хариу={body}"
                ),
                Err(e) => error!("❌ [{tag_s} SAMBAR] ip={ip_s} aldaa={e}"),
            }
        });
    }

    if !command_id.is_empty() {
        let result_payload = serde_json::json!({
            "commandId": command_id,
            "ip": ip,
            "success": result.is_ok(),
            "error": result.err(),
        });
        let _ = socket.emit("execute-open-result", result_payload);
    }
}

/// Сервер тооцоолсон төлбөрийн дүнг гарцын самбарт гаргана.
///
/// Дүн нь зөвхөн серверт мэдэгддэг (зочин, гэрээт гэх мэт төрөл бүрийн
/// тооцоо тэнд хийгддэг) тул ажилтан нь зүгээр л дамжуулагч: ирсэн
/// дугаар, дүнг нь хэвээр нь бичнэ.
fn handle_sambar_show(payload: Payload, tag: &str) {
    let json_str = match payload {
        Payload::String(s) => s,
        Payload::Binary(b) => String::from_utf8_lossy(&b).to_string(),
    };

    let v: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            error!("❌ [{tag} SAMBAR] JSON задлахад алдаа: {json_str} ({e})");
            return;
        }
    };

    let ip = v.get("ip").and_then(|i| i.as_str()).unwrap_or_default().to_string();
    let plate = v.get("plate").and_then(|p| p.as_str()).unwrap_or_default().to_string();
    let dun = v.get("dun").and_then(|d| d.as_str()).unwrap_or("0").to_string();
    let turul = v.get("turul").and_then(|t| t.as_str()).unwrap_or_default().to_string();

    if ip.is_empty() {
        error!("❌ [{tag} SAMBAR] 'ip' талбар дутуу байна");
        return;
    }

    info!("🔆 [{tag} SAMBAR] Received: ip={ip} plate={plate} turul={turul} dun={dun}");

    let tag_s = tag.to_string();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                error!("❌ [{tag_s} SAMBAR] runtime үүсгэж чадсангүй: {e}");
                return;
            }
        };
        // Дөрвөн мөр: дугаар / төрөл / дүн / байгууллага.
        let dun_t = format!("{dun}T");
        let baig = CAMERA_MANAGER
            .get()
            .map(|m| m.org_name().to_string())
            .unwrap_or_default();
        // let murnuud: Vec<&str> = vec![&plate, &turul, &dun_t, &baig];

        match rt.block_on(crate::api::sambar_exit(&ip, &plate, &turul, &dun)) {
            Ok(body) => info!(
                "🔆 [{tag_s} SAMBAR] бичигдлээ turul={turul} dun={dun} хариу={body}"
            ),
            Err(e) => error!("❌ [{tag_s} SAMBAR] ip={ip} aldaa={e}"),
        }
    });
}

/// Урсгалыг шаардлагаар асаах/зогсоох дохио.
///
/// ── Гинж ────────────────────────────────────────────────────────────────
/// хөтөч WHEP хүсэлт → MediaMTX дээр зам хоосон → `runOnDemand` скрипт
/// → sukhBackv2 → socket (`gate-room-{barilgiinId}`) → ЭНД → ffmpeg асна.
///
/// Урсгал бүрийг 24 цаг дамжуулах нь барилгын шугамыг дүүргэдэг (хэмжилтээр
/// 34 урсгал = 18 Mbps) тул зөвхөн бодит үзэгчтэйг нь асаана.
fn handle_urgats(payload: Payload, tag: &str, asaakh: bool) {
    let json_str = match payload {
        Payload::String(s) => s,
        Payload::Binary(b) => String::from_utf8_lossy(&b).to_string(),
    };

    // Backend нь `{"path":"<зам>"}` эсвэл зүгээр мөр илгээж болно.
    let zam = serde_json::from_str::<serde_json::Value>(&json_str)
        .ok()
        .and_then(|v| {
            v.get("path")
                .or_else(|| v.get("zam"))
                .and_then(|p| p.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| json_str.trim().trim_matches('"').to_string());

    if zam.is_empty() {
        error!("❌ [{tag} URGATS] зам хоосон: {json_str}");
        return;
    }

    if asaakh {
        info!("▶️  [{tag} URGATS] аса: {zam}");
        if !crate::publisher::urgats_asaa(&zam) {
            warn!("⚠️  [{tag} URGATS] {zam} — бүртгэгдээгүй зам");
        }
    } else {
        info!("⏹️  [{tag} URGATS] зогсоо: {zam}");
        crate::publisher::urgats_zogsoo(&zam);
    }
}

fn summarize_ice(sdp: &str) -> String {
    let mut kinds: Vec<String> = Vec::new();
    for line in sdp.lines() {
        let line = line.trim();
        if !line.starts_with("a=candidate:") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let addr = parts[4];
        let typ = parts.get(7).copied().unwrap_or("?");
        let label = if addr.ends_with(".local") { "host(mdns)" } else { typ };
        let entry = format!("{label}:{addr}");
        if !kinds.contains(&entry) {
            kinds.push(entry);
        }
    }
    if kinds.is_empty() {
        "NONE".to_string()
    } else {
        kinds.join(" ")
    }
}

fn decode_sdp(s: &str) -> String {
    let s = s.trim();

    let inner = serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| {
            v.get("sdp64")
                .or_else(|| v.get("sdp"))
                .and_then(|f| f.as_str())
                .map(|f| f.to_string())
        })
        .unwrap_or_else(|| s.to_string());

    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(inner.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or(inner)
}

fn rewrite_rtsp_url(url: &str) -> String {
    // Parse Dahua /Streaming/Channels/XXYY or /cam/realmonitor format
    // For smooth WebRTC playback, always ensure subtype=1 (substream)
    if let Some(r_pos) = url.find("rtsp://") {
        let after_rtsp = &url[r_pos + 7..];
        if let Some(at_pos) = after_rtsp.find('@') {
            let credentials = &after_rtsp[..at_pos];
            let rest = &after_rtsp[at_pos + 1..];

            // Extract IP (up to ':' or '/')
            let ip_end = rest.find(':').or_else(|| rest.find('/')).unwrap_or(rest.len());
            let ip = &rest[..ip_end];

            // Танигдсан Dahua төхөөрөмж биш бол ХӨНДӨХГҮЙ.
            // `/Streaming/Channels/NNN` нь жинхэнэ Hikvision-ы зам байж болно.
            let Some(tokhirsan_zam) = dahua_zam(ip) else {
                return url.to_string();
            };

            // `[[cameras]].root` заасан бол ЯГ ТЭРИЙГ хэрэглэнэ. Зарим ANPR
            // камер `cam/realmonitor` -аар үйлчилдэггүй (`/live` гэх мэт) —
            // хүчээр хөрвүүлбэл r2w зураг авчрахгүй, хар дэлгэц гарна.
            if !tokhirsan_zam.is_empty() {
                return format!("rtsp://{credentials}@{ip}:554/{tokhirsan_zam}");
            }

            // Хөрвүүлэлтийг ЗӨВХӨН Hikvision хэлбэрийн зам дээр хийнэ.
            //
            // Энэ засвар нь вэбд Dahua төхөөрөмжийг `Streaming/Channels/NNN`
            // гэж БУРУУ бичсэн тохиолдлыг нөхөх гэж үүссэн. Харин вэб дээр
            // зам аль хэдийн ЗӨВ бичигдсэн байвал (жишээ нь `live`) түүнийг
            // дарж бичих нь эвддэг: r2w зураг авчрахгүй, сигналчлал 200
            // байгаад хар дэлгэц гарна.
            //
            // Вэбийн тохиргоо нь эх сурвалж — түүнд гар хүрэхгүй.
            if !url.contains("/Streaming/Channels/") {
                return url.to_string();
            }

            let channel = if let Some(ch_pos) = url.find("/Streaming/Channels/") {
                let ch_str = &url[ch_pos + "/Streaming/Channels/".len()..];
                let ch_str = ch_str.split('/').next().unwrap_or(ch_str);
                let ch_str = ch_str.split('?').next().unwrap_or(ch_str);
                if ch_str.len() >= 2 {
                    let ch_num: u32 = ch_str[..ch_str.len() - 2].parse().unwrap_or(1);
                    ch_num.max(1)
                } else {
                    1
                }
            } else if let Some(cam_pos) = url.find("/cam/realmonitor") {
                let query = &url[cam_pos..];
                if let Some(ch_idx) = query.find("channel=") {
                    let after_ch = &query[ch_idx + 8..];
                    let ch_str = after_ch.split('&').next().unwrap_or("1");
                    ch_str.parse().unwrap_or(1)
                } else {
                    1
                }
            } else {
                1
            };

            // Always use subtype=1 (substream) for smooth, low latency WebRTC streaming!
            let rewritten = format!(
                "rtsp://{}@{}:554/cam/realmonitor?channel={}&subtype=1",
                credentials, ip, channel
            );
            return rewritten;
        }
    }
    url.to_string()
}

fn handle_webrtc_offer(payload: Payload, socket: RawClient, tag: &str) {
    let json_str = match payload {
        Payload::String(s) => s,
        Payload::Binary(b) => String::from_utf8_lossy(&b).to_string(),
    };

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) {
        let correlation_id = v.get("correlationId").and_then(|c| c.as_str()).unwrap_or_default().to_string();
        let raw_rtsp_url = v.get("rtspUrl").and_then(|r| r.as_str()).unwrap_or_default().to_string();
        let sdp64 = v.get("sdp64").and_then(|s| s.as_str()).unwrap_or_default().to_string();

        if correlation_id.is_empty() { return; }

        let rtsp_url = rewrite_rtsp_url(&raw_rtsp_url);
        let short_id: String = correlation_id.chars().take(8).collect();
        info!("📹 [{tag} STREAM {short_id}] offer browser-ice: {}", summarize_ice(&decode_sdp(&sdp64)));
        info!("📹 [{tag} STREAM {short_id}] rtsp: {rtsp_url}");

        // Request immediate hardware keyframe generation to eliminate initial startup black screen
        if let Some(r_pos) = raw_rtsp_url.find("rtsp://") {
            let after_rtsp = &raw_rtsp_url[r_pos + 7..];
            if let Some(at_pos) = after_rtsp.find('@') {
                let rest = &after_rtsp[at_pos + 1..];
                let ip_end = rest.find(':').or_else(|| rest.find('/')).unwrap_or(rest.len());
                let ip = &rest[..ip_end];
                if let Some(mgr) = CAMERA_MANAGER.get() {
                    let _ = mgr.make_key_frame(ip, 1, 1);
                }
            }
        }

        let tag_thread = tag.to_string();
        std::thread::spawn(move || {
            let local_r2w_url = "http://127.0.0.1:8083/stream";
            let params = [
                ("url", rtsp_url.as_str()),
                ("sdp64", sdp64.as_str()),
                ("sdp", sdp64.as_str()),
                ("tracks", "video")
            ];

            match HTTP_CLIENT.post(local_r2w_url).form(&params).send() {
                Ok(response) => {
                    let status = response.status();
                    if let Ok(sdp_answer) = response.text() {
                        if status.is_success() {
                            let answer_payload = serde_json::json!({
                                "correlationId": correlation_id,
                                "sdpAnswer": sdp_answer
                            });
                            let decoded = decode_sdp(&sdp_answer);
                            let ice = summarize_ice(&decoded);
                            info!("✅ [{tag_thread} STREAM {short_id}] answer server-ice: {ice}");
                            let _ = socket.emit("webrtc-answer", answer_payload);
                        } else {
                            error!("❌ [{tag_thread} STREAM {short_id}] R2W error ({status}): {sdp_answer}");
                            let _ = socket.emit("webrtc-answer", serde_json::json!({
                                "correlationId": correlation_id,
                                "error": sdp_answer
                            }));
                        }
                    }
                }
                Err(e) => {
                    error!("❌ [{tag_thread} STREAM {short_id}] cannot reach local R2W on :8083 — is it running? ({e})");
                    let _ = socket.emit("webrtc-answer", serde_json::json!({
                        "correlationId": correlation_id,
                        "error": e.to_string()
                    }));
                }
            }
        });
    }
}

