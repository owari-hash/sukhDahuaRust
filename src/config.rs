use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct BarilgaConfig {
    #[serde(rename = "barilgiinId")]
    pub barilgiin_id: String,
    #[serde(default)]
    pub ner: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BarilgaTarget {
    pub barilgiin_id: String,
    pub label: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server:     ServerConfig,
    pub sdk:        SdkConfig,
    pub cameras:    Vec<CameraEntry>,
    #[serde(default)]
    pub sambar_only: bool,
    #[serde(default, alias = "barilguud")]
    pub barilga:    Vec<BarilgaConfig>,
    /// `[publish]` хэсэг байвал камер бүрийг VPS рүү тасралтгүй нийтэлнэ.
    #[serde(default)]
    pub publish:    Option<PublishConfig>,
    /// `[[publish_extra]]` — зөвхөн урсгал нийтлэх нэмэлт камерууд.
    #[serde(default, rename = "publish_extra")]
    pub publish_extra: Vec<NemeltUrgats>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub url:              String,
    pub token:            String,
    #[serde(rename = "barilgiinId")]
    pub barilgiin_id:     String,
    pub timeout_secs:     u64,
    pub retry_count:      u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SdkConfig {
    pub username:                String,
    pub heartbeat_interval_secs: u64,
    pub connect_timeout_ms:      u32,
    pub max_connect_retries:     u32,
    pub port:                    u16,   // default 37777
    pub org_name:                String,
    pub company_name:            String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CameraEntry {
    pub ip:       String,
    pub password: String,
    pub http_port: Option<u16>,
    pub gate:      Option<String>,
    pub sambar_type: Option<String>,
    /// Энэ камер аль барилгад хамаарах. Заагаагүй бол `[server].barilgiinId`.
    ///
    /// Урсгалын зам нь `{barilgiinId}/{ip}` болдог тул хөтөч зөвхөн эдгээр
    /// хоёрыг мэдэж байхад л WHEP хаягийг өөрөө бодож чадна — шинэ талбар
    /// эсвэл шинэ API хэрэггүй.
    #[serde(rename = "barilgiinId")]
    pub barilgiin_id: Option<String>,
    /// RTSP-ийн ЗАМ (хаягын `/`-ээс хойш). Заагаагүй бол
    /// `cam/realmonitor?channel=1&subtype=1` (Dahua-гийн стандарт).
    ///
    /// Бүх Dahua төхөөрөмж тэр замаар үйлчилдэггүй: зарим ANPR
    /// марк `/live`, `/stream1` гэх мэт болно. Тэр үед хүчээр
    /// хөрвүүлбэл ffmpeg болон r2w хоёуланг буруу зам рүү
    /// орч хар дэлгэц гарна — сигналчлал 200 байсан ч.
    ///
    /// Жишээ: `root = "live"`
    #[serde(default)]
    pub root: Option<String>,
    /// Заавал бол НИЙТЛЭХ урсгалыг энэ өргөн рүү буулгана (пиксел).
    ///
    /// Заагаагүй бол `-c:v copy` — дахин кодчилдоггүй, CPU бараг тэг.
    ///
    /// Яагаад шаардлагатай: зарим ANPR камер substream өгдөггүй бөгөөд
    /// үндсэн урсгал нь 2688×1520 шиг том байдаг. Тэрийг барилгын
    /// upload-аар тэр чигээр дамжуулбал тогтохгүй (`source has timed out`).
    ///
    /// Камерын тохиргоог багасгахаас энэ нь АЮУЛГҮЙ: дугаар таних
    /// алгоритм камер дээрээ үндсэн урсгал/сенсор дээр гүйдэг тул
    /// тэрийг буулгавал танилт муудаж магадгүй. Энд зөвхөн хөтөч рүү
    /// явах хуулбар багасна.
    ///
    /// Жишээ: `urgun = 1280`
    #[serde(default)]
    pub urgun: Option<u32>,
}

/// SDK-д хамаарахгүй, ЗӨВХӨН урсгал нийтлэх камер.
///
/// Зогсоолын ANPR камерууд `[[cameras]]`-д бичигддэг: тэдэн рүү SDK-гаар
/// нэвтэрч, хаалга удирдаж, дугаар сонсдог. Харин СӨХ-ийн ерөнхий хяналтын
/// камерууд (вэбийн `/camera` хуудас) тийм биш — зөвхөн дүрс дамжуулна.
/// Тэднийг `[[cameras]]`-д хийвэл worker дэмий SDK нэвтрэлт, дугаар сонсогч
/// асааж, камерыг ачаалуулна.
#[derive(Debug, Deserialize, Clone)]
pub struct NemeltUrgats {
    /// Бүтэн RTSP хаяг — нэвтрэх нэр, порт, зам бүгд дотроо.
    /// Вэб ч мөн ижил хаягийг ашигладаг тул IP-г хоёр тал ижилхэн салгана.
    pub rtsp: String,
    /// Заагаагүй бол `[server].barilgiinId`.
    #[serde(rename = "barilgiinId")]
    pub barilgiin_id: Option<String>,
}

/// Камерын урсгалыг VPS дээрх MediaMTX рүү тасралтгүй нийтлэх тохиргоо.
///
/// Байхгүй бол нийтлэх хэсэг огт асахгүй — хуучин зан төлөв хэвээр.
#[derive(Debug, Deserialize, Clone)]
pub struct PublishConfig {
    /// Жишээ: `rtsp://publisher:НУУЦҮГ@103.236.194.99:8554`
    pub url: String,
    /// Камерын RTSP substream портыг дарж бичих (үндсэн 554).
    #[serde(default = "default_rtsp_port")]
    pub camera_rtsp_port: u16,
    /// `ffmpeg` бинарын зам. PATH дээр байвал зүгээр `ffmpeg`.
    #[serde(default = "default_ffmpeg")]
    pub ffmpeg: String,
    /// Тухайн камерыг нийтлэхгүй байх бол IP-г энд жагсаана.
    #[serde(default)]
    pub skip_ips: Vec<String>,
}

fn default_rtsp_port() -> u16 { 554 }
fn default_ffmpeg() -> String { "ffmpeg".to_string() }

impl Config {
    /// Камер аль барилгад хамаарах — тусад нь заагаагүй бол үндсэнийх.
    pub fn camera_barilga<'a>(&'a self, cam: &'a CameraEntry) -> &'a str {
        cam.barilgiin_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.server.barilgiin_id.trim())
    }

    /// MediaMTX дээрх урсгалын зам: `{barilgiinId}/{ip-цэггүй}`.
    ///
    /// Хөтөч нь `barilgiinId` болон камерын IP хоёрыг аль хэдийн мэддэг тул
    /// WHEP хаягийг ижил дүрмээр өөрөө бодно — шинэ API хэрэггүй.
    /// Цэгийг зураасаар солив: зарим прокси зам доторх цэгийг файлын
    /// өргөтгөл мэт ойлгодог.
    pub fn stream_path(&self, cam: &CameraEntry) -> String {
        format!("{}/{}", self.camera_barilga(cam), cam.ip.replace('.', "-"))
    }

    /// Нэмэлт урсгалын зам. IP болон сувгийг RTSP хаягаас салгана — вэб
    /// талын `urgasniiZam` -тай яг ижил дүрэм.
    pub fn nemelt_zam(&self, u: &NemeltUrgats) -> Option<String> {
        let ip = rtsp_ip(&u.rtsp)?;
        let bid = u
            .barilgiin_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.server.barilgiin_id.trim());
        if bid.is_empty() {
            return None;
        }
        let dagavar = match rtsp_suvag(&u.rtsp) {
            Some(s) => format!("-{s}"),
            None => String::new(),
        };
        Some(format!("{bid}/{}{dagavar}", ip.replace('.', "-")))
    }

    pub fn load() -> anyhow::Result<Self> {
        let exe_dir = std::env::current_exe()?
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_owned();
        let cfg_path: PathBuf = exe_dir.join("config.toml");
        let text = std::fs::read_to_string(&cfg_path)
            .map_err(|e| anyhow::anyhow!("Cannot read {}: {e}", cfg_path.display()))?;
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("Config parse error: {e}"))
    }

    /// Returns all building targets (id, label) to bridge.
    /// Combines primary `server.barilgiin_id` and all `barilga` entries, deduplicated.
    pub fn targets(&self) -> Vec<BarilgaTarget> {
        let mut list: Vec<BarilgaTarget> = Vec::new();

        let primary_id = self.server.barilgiin_id.trim();
        if !primary_id.is_empty() {
            let label = self.sdk.org_name.trim();
            let label_str = if label.is_empty() { primary_id.to_string() } else { label.to_string() };
            list.push(BarilgaTarget {
                barilgiin_id: primary_id.to_string(),
                label: label_str,
            });
        }

        for b in &self.barilga {
            let id = b.barilgiin_id.trim();
            if id.is_empty() { continue; }
            if !list.iter().any(|t| t.barilgiin_id == id) {
                let label = b.ner.as_deref().unwrap_or(id).trim().to_string();
                list.push(BarilgaTarget {
                    barilgiin_id: id.to_string(),
                    label,
                });
            }
        }

        list
    }
}

/// `rtsp://user:pass@192.168.1.60:554/...` → `192.168.1.60`
///
/// Нэвтрэх хэсэг нь `@`-г агуулж болохгүй (URL-д percent-encode хийнэ) тул
/// СҮҮЛИЙН `@`-аас хойших хэсгийг хост гэж үзнэ.
pub fn rtsp_ip(url: &str) -> Option<String> {
    let s = url.trim();
    // Зөвхөн RTSP. Вэб/апп `^rtsps?://`-г шалгадаг тул
    // схем буруу байвал тэд зам бодохгүй. Манай тал бодвол
    // хөтөр хүсдэггүй зам руу цараа ffmpeg асна.
    let doorkh = s.to_ascii_lowercase();
    if !doorkh.starts_with("rtsp://") && !doorkh.starts_with("rtsps://") {
        return None;
    }
    let after = s.split("://").nth(1)?;
    let host_part = match after.rfind('@') {
        Some(i) => &after[i + 1..],
        None => after,
    };
    let end = host_part
        .find(|c| c == ':' || c == '/' || c == '?' || c == '#')
        .unwrap_or(host_part.len());
    let ip = &host_part[..end];
    if ip.is_empty() { None } else { Some(ip.to_string()) }
}

/// RTSP хаягаас СУВГИЙН дугаарыг салгана.
///
/// ── Яагаад хэрэгтэй вэ ──────────────────────────────────────────────────
/// NVR бол НЭГ IP дээр олон камер. Зөвхөн IP-гээр зам нэрлэвэл бүх суваг
/// нэг зам руу нийтлэхийг оролдож, бие биенээ түлхэнэ.
///
/// Зогсоолын ANPR камерын `root` нь `tokhirgoo.ROOT || "stream"` бөгөөд
/// сувгийн дугаар агуулдаггүй. Тиймээс "суваг олдвол л дагавар нэмэх"
/// дүрэм нь одоо ажиллаж байгаа замуудыг ХЭВЭЭР үлдээнэ.
///
/// `Streaming/Channels/102` → `102`
/// `cam/realmonitor?channel=2&subtype=1` → `2`
/// `stream` → None
pub fn rtsp_suvag(url: &str) -> Option<String> {
    let u = url.trim();

    // Hikvision маягийн зам
    if let Some(i) = u.find("/Channels/") {
        let uldsen = &u[i + "/Channels/".len()..];
        let dugaar: String = uldsen.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !dugaar.is_empty() {
            return Some(dugaar);
        }
    }

    // Query параметр: ?channel=N эсвэл &channel=N
    for tusgaarlagch in ["?channel=", "&channel="] {
        if let Some(i) = u.find(tusgaarlagch) {
            let uldsen = &u[i + tusgaarlagch.len()..];
            let dugaar: String = uldsen.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !dugaar.is_empty() {
                return Some(dugaar);
            }
        }
    }

    None
}
