//! Posts plate events to the Node.js server.

use std::time::Duration;
use log::{warn, error};
use reqwest::Client;
use serde::Serialize;
use tokio::time::sleep;

use crate::config::ServerConfig;
use crate::camera_manager::PlateEvent;

#[derive(Serialize)]
pub struct PlatePayload<'a> {
    pub mashiniiDugaar: &'a str,
    pub CAMERA_IP:      &'a str,
    #[serde(rename = "barilgiinId")]
    pub barilgiin_id:   &'a str,
}

pub struct PlateService {
    client: Client,
    cfg:    ServerConfig,
}

impl PlateService {
    pub fn new(cfg: ServerConfig) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .build()?;
        Ok(Self { client, cfg })
    }

    pub async fn process_plate(&self, event: &PlateEvent) {
        let payload = PlatePayload {
            mashiniiDugaar: &event.plate,
            CAMERA_IP:      &event.camera_ip,
            barilgiin_id:   &self.cfg.barilgiin_id,
        };

        match self.send_with_retry(&payload).await {
            Ok(_)  => {},
            Err(e) => log::error!("PLATE FAIL | plate={} camera={} err={e}", event.plate, event.camera_ip),
        }
    }

    async fn send_with_retry(&self, payload: &PlatePayload<'_>) -> anyhow::Result<()> {
        let url   = &self.cfg.url;
        let token = &self.cfg.token;
        let max   = self.cfg.retry_count;

        for attempt in 0..=max {
            match self.send_once(url, token, payload).await {
                Ok(_)  => return Ok(()),
                Err(e) if attempt < max => {
                    let delay = 2_u64.pow(attempt + 1);
                    log::error!("PLATE RETRY | attempt={}/{max} plate={} err={e} retry_in={delay}s",
                        attempt + 1, payload.mashiniiDugaar);
                    sleep(Duration::from_secs(delay)).await;
                }
                Err(e) => {
                    log::error!("PLATE GIVE UP | plate={} all {max} attempts failed err={e}",
                        payload.mashiniiDugaar);
                    return Err(e);
                }
            }
        }
        unreachable!()
    }

    async fn send_once(&self, url: &str, token: &str, payload: &PlatePayload<'_>) -> anyhow::Result<()> {
        let resp = self.client
            .post(url)
            .bearer_auth(token)
            .json(payload)
            .send()
            .await?;

        let status = resp.status();
        let text   = resp.text().await.unwrap_or_default();
        // println! нь fern-ээр баригддаггүй тул Windows үйлчилгээ болж
        // ажиллахад энэ мөр ХАА Ч харагддаггүй байв.
        log::info!("Server raw response ({status}): {text}");

        // Сервер 200 буцаасан ч `aldaa` талбартай бол машиныг ОРУУЛААГҮЙ
        // гэсэн үг: хаалга нээгдэхгүй тул самбар өмнөх машины дугаар дээрээ
        // хөшиж үлддэг байв. Жолоочид шалтгааныг нь харуулна — серверийн
        // ӨӨРИЙНХ нь үгээр, энд орчуулга зохиохгүй.
        if status.is_success() {
            let json_val: Option<serde_json::Value> = serde_json::from_str(&text).ok();
            let dugaar = payload.mashiniiDugaar.to_string();
            let ip = payload.CAMERA_IP.to_string();

            if let Some(aldaa) = json_val.as_ref().and_then(|v| v.get("aldaa").and_then(|a| a.as_str())) {
                warn!("PLATE DENIED | plate={dugaar} camera={ip} shaltgaan={aldaa}");
                if let Err(e) = crate::api::sambar_burtgelgui(&ip, &dugaar).await {
                    error!("❌ [SAMBAR] бүртгэлгүй машин бичиж чадсангүй: {e}");
                }
            } else {
                let turul = json_val.as_ref()
                    .and_then(|v| v.get("turul").and_then(|t| t.as_str()))
                    .unwrap_or("Оршин суугч");
                log::info!("PLATE APPROVED | plate={dugaar} camera={ip} turul={turul}");
                if let Err(e) = crate::api::sambar_registered(&ip, &dugaar, turul).await {
                    error!("❌ [SAMBAR] бүртгэлтэй машин бичиж чадсангүй: {e}");
                }
            }
        }

        if !status.is_success() {
            anyhow::bail!("Server returned {status}: {text}");
        }
        Ok(())
    }
}
