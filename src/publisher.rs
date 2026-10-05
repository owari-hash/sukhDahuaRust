//! Камерын урсгалыг VPS дээрх MediaMTX рүү ТАСРАЛТГҮЙ нийтлэх.
//!
//! ── Яагаад ──────────────────────────────────────────────────────────────
//! Өмнө нь үзэгч бүр өөрийн WebRTC холболтыг ЭНЭ компьютер РҮҮ шууд
//! (P2P) үүсгэдэг байв. Хоёр тал NAT-ын ард байдаг тул холболт нь TURN-гүй
//! бол зарчмын хувьд үүсэхгүй, мөн үзэгч бүр камер руу ТУСДАА RTSP сесс
//! нээдэг байсан — камерын сессийн хязгаар хурдан дүүрдэг.
//!
//! Одоо камер бүрийг НЭГ л удаа нийтийн сервер рүү түлхэнэ. Үзэгчид тэндээс
//! татна: нийтийн тогтмол IP тул ICE үргэлж бүтнэ, камер дээрх ачаалал
//! үзэгчийн тооноос хамаарахаа болино.
//!
//! ── Яагаад ffmpeg ───────────────────────────────────────────────────────
//! `-c:v copy` — дахин кодчилол ХИЙХГҮЙ, зөвхөн пакет дамжуулна. CPU бараг
//! тэг. Дуу хаягдана (`-an`): Dahua нь ихэвчлэн G.711/AAC өгдөг бөгөөд
//! WebRTC түүнийг шууд дамжуулж чаддаггүй тул үлдээвэл зарим хөтөч дээр
//! урсгал огт эхлэхгүй байх эрсдэлтэй.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use log::{error, info, warn};
use once_cell::sync::Lazy;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::config::{CameraEntry, Config, PublishConfig};

/// Дараалан унасан тохиолдолд хүлээх хугацааны дээд хязгаар.
const MAX_BACKOFF_SECS: u64 = 30;

/// Нийтлэгчийг шатлан асаах зай (мс).
///
/// ── Яагаад ──────────────────────────────────────────────────────────────
/// NVR-ын бүх суваг НЭГ төхөөрөмж дээр байдаг. Хэрэв 16 ffmpeg нэг секундэд
/// зэрэг нэвтрэхийг оролдвол Dahua сессийн хязгаараа хэтрүүлж `401
/// Unauthorized` буцаана — нэвтрэлт зөв байсан ч. Улмаар давтан оролдлого
/// нь төхөөрөмжийг түр хаалтад оруулж, гогцоо үүсгэнэ.
///
/// Шаталж асаавал нэвтрэлтүүд тархаж, хязгаарт хүрэхгүй.
const KHELTES_MS: u64 = 600;

/// Нэвтрэлтийн алдааны дараа хүлээх хугацаа (секунд).
///
/// ── Яагаад тусдаа, УРТ хугацаа ──────────────────────────────────────────
/// Dahua/Hikvision төхөөрөмж нь дараалсан буруу нэвтрэлтийн дараа эрхийг
/// ТҮР ХААДАГ. Тэр үед ЗӨВ нууц үг ч `401` буцаана.
///
/// Ердийн ухралт (1→4→8→16→30с) нь нэвтрэлтийн алдаанд тохиромжгүй: найман
/// суваг тус бүр минутад хэдэн арван удаа оролдож, цоожны хугацааг
/// тасралтгүй шинэчилж, төхөөрөмж ХЭЗЭЭ Ч тайлагдахгүй болно. Өөрөөр
/// хэлбэл worker өөрөө өөрийгөө цоожилно.
///
/// Нэвтрэлтийн алдаа нь түр зуурын зүйл биш — тохиргоо солигдохоос нааш
/// хариу өөрчлөгдөхгүй. Тиймээс урт хүлээлт нь цоож тайлагдах зай гаргаж,
/// вэб дээр нууц үг зассаны дараа 5 минутын дотор ӨӨРӨӨ сэргэнэ
/// (үйлчилгээг дахин асаах шаардлагагүй).
const NEVTRELT_UKHRALT_SECS: u64 = 300;

/// Хамгийн сүүлд ЗАХИАЛСАН асаалтын мөч.
///
/// Өмнө нь энэ нь «хэд дэх нийтлэгч» гэсэн тоолуур байв (`n * KHELTES_MS`).
/// Тэр тоолуур ХЭЗЭЭ Ч ТЭГШИРДЭГГҮЙ тул хүлээлт төгсгөлгүй өснө: процесс
/// 50 урсгал асаасан байвал 51 дэх нь ЦОРЫН НЭГ асаалт байсан ч 30 секунд
/// хүлээдэг байсан. Логт `аса:` → `эхэллээ` зөрүү 0-ээс 9 секунд болж
/// өссөн нь яг үүнээс.
///
/// Одоо цаг захиалдаг: асаалт бүр өмнөхөөсөө `KHELTES_MS`-ээр хойш суудаг
/// бөгөөд өмнөх захиалга АЛЬ ХЭДИЙН өнгөрсөн бол шууд эхэлнэ. Иймд зэрэг
/// гарсан бөөгнөрөл л тарж, тайван үед хүлээлт тэг болно.
static SUULIIN_ZAKHIALGA: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

/// Шатлалын зайг ЗАХИАЛЖ, хүлээх хугацааг буцаана.
///
/// Mutex-ийг зөвхөн бодолтын хооронд эзэмшинэ — унтаж байх зуур бусад
/// урсгалыг гацаахгүй.
fn kheltes_zakhialya() -> Duration {
    let odoo = Instant::now();
    let mut suuliin = match SUULIIN_ZAKHIALGA.lock() {
        Ok(x) => x,
        Err(_) => return Duration::ZERO,
    };

    let suuri = match *suuliin {
        Some(t) if t > odoo => t,
        _ => odoo,
    };
    let minii = suuri + Duration::from_millis(KHELTES_MS);
    *suuliin = Some(minii);

    minii.saturating_duration_since(odoo)
}

/// Процесс ЭНЭ хугацаанаас удаан ажилласан бол "тогтвортой байсан" гэж үзэж
/// дахин унахад хүлээлтийг тэглэнэ. Үгүй бол нэг өдөр ажиллаад унасан
/// урсгал 30 секунд хүлээх шалтгаангүй.
const TOGTVORTOI_SECS: u64 = 60;

/// Хүүхэд процессуудыг эцэгтэйгээ хамт үхдэг болгох.
///
/// ── Яагаад хэрэгтэй вэ ──────────────────────────────────────────────────
/// Windows дээр эцэг процесс зогсоход хүүхэд нь АВТОМАТААР үхдэггүй.
/// Үйлчилгээг дахин асаах бүрд хуучин ffmpeg-үүд үлдэж хоцорвол:
///   • камерын RTSP сессийг барьсаар байна (Dahua-гийн хязгаар дүүрнэ)
///   • MediaMTX дээрх замыг эзэлсэн хэвээр тул ШИНЭ ffmpeg нийтэлж чадахгүй
/// Үр дүнд нь үйлчилгээг дахин асаах тусам байдал улам дордоно.
///
/// Job Object дээр `KILL_ON_JOB_CLOSE` тавьбал үйлчилгээ ЯМАР Ч шалтгаанаар
/// (зогсоох, унах, дахин асаах) дуусахад цөм нь хүүхдүүдийг нь цэвэрлэнэ.
#[cfg(windows)]
mod ajiliin_bag {
    use once_cell::sync::OnceCell;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct Bag(HANDLE);
    // HANDLE нь ердөө тоо — урсгал хооронд дамжуулахад аюулгүй.
    unsafe impl Send for Bag {}
    unsafe impl Sync for Bag {}

    static BAG: OnceCell<Option<Bag>> = OnceCell::new();

    fn bag() -> Option<HANDLE> {
        BAG.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h == 0 {
                log::warn!("PUBLISH | Job Object үүсгэж чадсангүй — ffmpeg үлдэж хоцорч магадгүй");
                return None;
            }
            let mut medeelel: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            medeelel.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &mut medeelel as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                log::warn!("PUBLISH | Job Object тохируулж чадсангүй");
                return None;
            }
            Some(Bag(h))
        })
        .as_ref()
        .map(|b| b.0)
    }

    /// Шинэ хүүхдийг багт оруулна. Амжилтгүй болсон ч нийтлэл үргэлжилнэ.
    pub fn nemye(khuukhed: &std::process::Child) {
        if let Some(h) = bag() {
            unsafe {
                AssignProcessToJobObject(h, khuukhed.as_raw_handle() as HANDLE);
            }
        }
    }
}

#[cfg(not(windows))]
mod ajiliin_bag {
    pub fn nemye(_khuukhed: &std::process::Child) {}
}

/// Нийтлэгчийг асаана. `[publish]` тохиргоо байхгүй бол юу ч хийхгүй.
// ═════════════════════════════════════════════════════════════════════════
//  Урсгалын бүртгэл — «боломжтой» ба «ажиллаж байгаа» хоёрыг САЛГАНА
// ═════════════════════════════════════════════════════════════════════════
//
//  ── Яагаад ──────────────────────────────────────────────────────────────
//  Хэмжилт: нэг NVR-ын 8 суваг ДӨРВӨН барилгад бүртгэгдсэн тул 32 урсгал
//  болж, барилгаас 18 Mbps тасралтгүй гарч, шугам дүүрч, саатал минутаар
//  хуримтлагдаж байв. Гэвч бодит үзэгч зэрэг 1-2 камер хардаг.
//
//  Тиймээс камер бүрийг УРЬДЧИЛЖ бүртгээд, ffmpeg-ийг зөвхөн үзэгч гарч
//  ирэхэд асаана. MediaMTX-ийн `runOnDemand` нь уншигч ирэхэд backend-д
//  хэлж, backend socket-оор энэ рүү `urgats-start` илгээнэ.
//
//  Хаалганы камерууд (`[[cameras]]`) ба гараар бичсэн `[[publish_extra]]`
//  нь БАЙНГА ажиллана — операторууд тэднийг тасралтгүй хардаг.

/// Нэг урсгалыг асаахад шаардлагатай бүх зүйл.
#[derive(Clone)]
struct UrgatsTokhirgoo {
    shoshgo: String,
    esh: String,
    zorilt: String,
    ffmpeg: String,
    /// `true` — БАЙНГА ажиллана, үзэгчийн дохиогоор зогсохгүй.
    ///
    /// Хаалганы ANPR камеруудыг оператор тасралтгүй хардаг. Мөн worker
    /// дахин асахад тэдгээрийн зам хэсэг хугацаанд хоосон байдаг тул
    /// MediaMTX `runOnDemand`-ыг ТЭДЭН ДЭЭР ч ажиллуулж, 20 секундын дараа
    /// `stop` илгээж байсан — байнгын урсгалыг унтраадаг байв.
    baingiin: bool,
    /// Заавал бол нийтлэх урсгалыг энэ өргөн рүү буулгаж КОДЧИЛНО.
    /// `None` — `-c:v copy`, дахин кодчилохгүй.
    urgun: Option<u32>,
}

/// Зам → тохиргоо. Нийтлэх БОЛОМЖТОЙ бүх урсгал.
static BOLOMJTOI: Lazy<Mutex<HashMap<String, UrgatsTokhirgoo>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Зам → «үргэлжлүүлэх» дохио. Ажиллаж БАЙГАА урсгалууд.
static AJILLAJ: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Урсгалыг бүртгэнэ (асаахгүй). Дараа `urgats_asaa`-гаар асна.
fn burtgeye(zam: &str, t: UrgatsTokhirgoo) -> bool {
    let mut b = match BOLOMJTOI.lock() {
        Ok(x) => x,
        Err(_) => return false,
    };
    if b.contains_key(zam) {
        return false;
    }
    b.insert(zam.to_string(), t);
    true
}

/// Шаардлагаар асаана. Аль хэдийн ажиллаж байвал юу ч хийхгүй.
///
/// `true` — асаалаа эсвэл аль хэдийн ажиллаж байна.
/// `false` — тийм зам бүртгэгдээгүй.
pub fn urgats_asaa(zam: &str) -> bool {
    {
        let a = match AJILLAJ.lock() {
            Ok(x) => x,
            Err(_) => return false,
        };
        if a.contains_key(zam) {
            return true;
        }
    }

    let t = match BOLOMJTOI.lock() {
        Ok(b) => match b.get(zam) {
            Some(t) => t.clone(),
            None => {
                warn!("PUBLISH | {zam} — бүртгэгдээгүй зам, асаах боломжгүй");
                return false;
            }
        },
        Err(_) => return false,
    };

    let yavuulakh = Arc::new(AtomicBool::new(true));
    if let Ok(mut a) = AJILLAJ.lock() {
        a.insert(zam.to_string(), yavuulakh.clone());
    }

    info!("PUBLISH | {} асаалаа", t.shoshgo);

    // Шатлал: нэг NVR рүү хэд хэдэн нэвтрэлт зэрэг цохивол Dahua сессийн
    // хязгаараа хэтрүүлж `401` буцаадаг. Асаалтын үед (байнгын урсгалууд)
    // тэд зэрэг гардаг тул тараана.
    let kheltes = kheltes_zakhialya();

    let shoshgo = t.shoshgo.clone();
    std::thread::Builder::new()
        .name(format!("publish-{shoshgo}"))
        .spawn(move || {
            if !kheltes.is_zero() {
                std::thread::sleep(kheltes);
            }
            kharuulakh(&t, &yavuulakh)
        })
        .ok();
    true
}

/// Үзэгч дуусахад зогсооно.
pub fn urgats_zogsoo(zam: &str) {
    // Байнгын урсгалыг үзэгчийн дохио ЗОГСООХ ЭРХГҮЙ.
    if let Ok(b) = BOLOMJTOI.lock() {
        if b.get(zam).map(|t| t.baingiin).unwrap_or(false) {
            info!("PUBLISH | {zam} — байнгын урсгал, зогсоохгүй");
            return;
        }
    }

    let dokhio = match AJILLAJ.lock() {
        Ok(mut a) => a.remove(zam),
        Err(_) => None,
    };
    match dokhio {
        Some(d) => {
            d.store(false, Ordering::SeqCst);
            info!("PUBLISH | {zam} зогсоох дохио илгээв");
        }
        None => info!("PUBLISH | {zam} — ажиллаагүй байсан"),
    }
}

/// Бүртгээд шууд асаана — байнга ажиллах урсгалд.
fn burtgeed_asaaya(zam: &str, t: UrgatsTokhirgoo) {
    if burtgeye(zam, t) {
        urgats_asaa(zam);
    } else {
        info!("PUBLISH | {zam} — давхардсан зам, алгаслаа");
    }
}

pub fn start(cfg: &Config) {
    let pub_cfg = match &cfg.publish {
        Some(p) if !p.url.trim().is_empty() => p.clone(),
        _ => {
            info!("PUBLISH | `[publish]` тохиргоо алга — урсгал нийтлэхгүй");
            return;
        }
    };

    let suuri = pub_cfg.url.trim().trim_end_matches('/').to_string();
    let khereglegch = cfg.sdk.username.clone();

    // Нэг зам ХОЁР удаа нийтлэгдвэл хоёр ffmpeg нэг зам руу өрсөлдөж,
    // MediaMTX эхнийхийг хөөнө. Иймд авсан замуудыг тэмдэглэнэ.
    let mut avsan: HashSet<String> = HashSet::new();

    // ── Зогсоолын ANPR камерууд ──────────────────────────────────────────
    for cam in &cfg.cameras {
        if pub_cfg.skip_ips.iter().any(|s| s.trim() == cam.ip) {
            info!("PUBLISH | {} — skip_ips дотор байна, алгаслаа", cam.ip);
            continue;
        }
        let zam = cfg.stream_path(cam);
        let esh = kameriin_rtsp(cam, &khereglegch, pub_cfg.camera_rtsp_port);
        urgats_asaaya(&pub_cfg, &suuri, &mut avsan, &zam, &esh, "", false, cam.urgun);
    }

    // ── Гараар бичсэн нэмэлт урсгалууд ───────────────────────────────────
    // Автомат хайлт бүтэхгүй үед (сүлжээ, токен) эдгээр нь нөөц болно.
    for nemelt in &cfg.publish_extra {
        match cfg.nemelt_zam(nemelt) {
            Some(zam) => urgats_asaaya(
                &pub_cfg, &suuri, &mut avsan, &zam, &nemelt.rtsp, "нэмэлт", false, None,
            ),
            None => warn!(
                "PUBLISH | нэмэлт урсгалын RTSP хаягаас IP салгаж чадсангүй: {}",
                nuutsgui(&nemelt.rtsp)
            ),
        }
    }

    // ── Серверээс БҮХ барилгын камерыг татах ─────────────────────────────
    // Тусдаа урсгалд: сүлжээ хүлээх нь үйлчилгээний асаалтыг тормозлох
    // ёсгүй. Мөн tokio runtime дотор биш — блоклодог HTTP клиент хэрэглэнэ.
    let cfg_khuulbar = cfg.clone();
    std::thread::Builder::new()
        .name("publish-discover".to_string())
        .spawn(move || kameruudiig_olyo(cfg_khuulbar, pub_cfg, suuri, avsan))
        .ok();
}

/// Урсгалыг бүртгэнэ. `shaardlagaar = false` бол шууд асаана.
///
/// Логийн шошго нь замын СҮҮЛИЙН хэсэг (ж: `192-168-1-100-2`). NVR-ын
/// сувгууд бүгд НЭГ IP дээр байдаг тул зөвхөн IP-гээр бичвэл ижил мөрүүд
/// гарч, алийг нь унасныг олох боломжгүй.
fn urgats_asaaya(
    pub_cfg: &PublishConfig,
    suuri: &str,
    avsan: &mut HashSet<String>,
    zam: &str,
    esh: &str,
    turul: &str,
    shaardlagaar: bool,
    urgun: Option<u32>,
) {
    if !avsan.insert(zam.to_string()) {
        info!("PUBLISH | {zam} — давхардсан зам, алгаслаа");
        return;
    }

    let shoshgo = zam.rsplit('/').next().unwrap_or(zam).to_string();
    let zorilt = format!("{suuri}/{zam}");

    if turul.is_empty() {
        info!("PUBLISH | {shoshgo} → {}", nuutsgui(&zorilt));
    } else {
        info!("PUBLISH | {shoshgo} ({turul}) → {}", nuutsgui(&zorilt));
    }

    let t = UrgatsTokhirgoo {
        shoshgo,
        esh: esh.to_string(),
        zorilt,
        ffmpeg: pub_cfg.ffmpeg.clone(),
        baingiin: !shaardlagaar,
        urgun,
    };

    if shaardlagaar {
        // Зөвхөн бүртгэнэ — үзэгч гарч ирэхэд `urgats_asaa` асаана.
        burtgeye(zam, t);
    } else {
        burtgeed_asaaya(zam, t);
    }
}

/// Камерын substream RTSP хаяг. Нууц үгийг URL-д аюулгүй болгож кодчилно —
/// `@` эсвэл `:` агуулсан нууц үг хаягийг эвддэг.
fn kameriin_rtsp(cam: &CameraEntry, khereglegch: &str, port: u16) -> String {
    let u = utf8_percent_encode(khereglegch, NON_ALPHANUMERIC).to_string();
    let p = utf8_percent_encode(&cam.password, NON_ALPHANUMERIC).to_string();
    let zam = kameriin_zam(cam);
    format!("rtsp://{u}:{p}@{}:{port}/{zam}", cam.ip)
}

/// Камерын RTSP зам. `[[cameras]].root` заасан бол тэр, үгүй бол Dahua-гийн
/// стандарт substream.
///
/// Бүх Dahua марк `cam/realmonitor` -аар үйлчилдэггүй: тааралдсан ANPR
/// камер `/live` -ээр үйлчилсэн. Хүчээр хөрвүүлбэл ffmpeg нийтлэж чадахгүй
/// (MediaMTX дээр `is publishing` гарахгүй), r2w ч зураг авчрахгүй —
/// сигналчлал 200 байсан ч хар дэлгэц.
pub fn kameriin_zam(cam: &CameraEntry) -> String {
    cam.root
        .as_deref()
        .map(str::trim)
        .map(|r| r.trim_start_matches('/'))
        .filter(|r| !r.is_empty())
        .unwrap_or("cam/realmonitor?channel=1&subtype=1")
        .to_string()
}

/// Hikvision хэлбэрийн замыг Dahua-гийн `cam/realmonitor` рүү хөрвүүлнэ.
///
/// ── Яагаад ──────────────────────────────────────────────────────────────
/// Вэбийн камерын тохиргоонд `root` нь `Streaming/Channels/102` гэж
/// хадгалагдсан байдаг ч төхөөрөмжүүд нь Dahua — тэр зам дээр `404`
/// буцаана. Хуучин P2P урсгал ажилладаг байсан нь `socket_bridge.rs`-ийн
/// `rewrite_rtsp_url` нь яг ижил хөрвүүлэлтийг хийдэг учраас.
///
/// Нийтлэгч ч ижлийг хийвэл вэбийн тохиргоонд ГАР ХҮРЭХ ШААРДЛАГАГҮЙ.
///
/// Сувгийн дугаар: Hikvision `NNYY` хэлбэрээс сүүлийн хоёр оронг хаяна
/// (`102` → 1, `802` → 8, `1601` → 16). `subtype=1` (дэд урсгал) үргэлж —
/// зурвас хэмнэх ба `rewrite_rtsp_url` -тэй адил байх.
///
/// ЧУХАЛ: зам нь ЭХНИЙ хаягаас бодогдоно (вэб мөн тэгдэг). Энд зөвхөн
/// ffmpeg-ийн ЭХ СУРВАЛЖ хөрвүүлэгдэнэ.
fn dahua_bolgoyo(url: &str, dahua_esekh: bool) -> String {
    let u = url.trim();

    // ЗӨВХӨН танигдсан Dahua хаалганы төхөөрөмж дээр хөрвүүлнэ.
    //
    // `/Streaming/Channels/NNN` бол ЖИНХЭНЭ Hikvision-ы зам. Вэб дээр
    // Dahua төхөөрөмжийг тэр хэлбэрээр буруу бичсэн тохиолдол байсан тул
    // энэ засвар үүссэн. Гэвч барилгад бодит Hikvision NVR байвал
    // хөрвүүлэх нь төгс замыг эвдэж, ffmpeg холбогдож чадахгүй.
    //
    // `[[cameras]]`-д бичигдсэн IP нь SDK-гаар нэвтрдэг Dahua ANPR
    // төхөөрөмж гэдэг нь батлагдсан — зөвхөн тэднийг засна.
    if !dahua_esekh {
        return u.to_string();
    }

    // Аль хэдийн Dahua хэлбэртэй бол хөндөхгүй.
    if u.contains("/cam/realmonitor") {
        return u.to_string();
    }

    let Some(skhem_tuk) = u.find("://") else { return u.to_string() };
    let daraa = &u[skhem_tuk + 3..];

    // Нэвтрэлт ба хост хэсгийг салгана.
    let (nevtrelt, khost_zam) = match daraa.rfind('@') {
        Some(i) => (Some(&daraa[..i]), &daraa[i + 1..]),
        None => (None, daraa),
    };

    let zam_tuk = khost_zam.find('/').unwrap_or(khost_zam.len());
    let khost = &khost_zam[..zam_tuk];
    let zam = &khost_zam[zam_tuk..];

    // Зөвхөн `/Streaming/Channels/NNN` хэлбэрийг хөрвүүлнэ. Бусад замыг
    // хөндөхгүй — жинхэнэ Hikvision төхөөрөмж байж магадгүй.
    let Some(i) = zam.find("/Channels/") else { return u.to_string() };
    let dugaar: String = zam[i + "/Channels/".len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if dugaar.len() < 3 {
        return u.to_string();
    }
    let suvag: u32 = dugaar[..dugaar.len() - 2].parse().unwrap_or(1);
    let suvag = suvag.max(1);

    // Портыг хадгална, байхгүй бол 554.
    let khost_port = if khost.contains(':') {
        khost.to_string()
    } else {
        format!("{khost}:554")
    };

    match nevtrelt {
        Some(n) => format!("rtsp://{n}@{khost_port}/cam/realmonitor?channel={suvag}&subtype=1"),
        None => format!("rtsp://{khost_port}/cam/realmonitor?channel={suvag}&subtype=1"),
    }
}

/// Логийн МӨР дотроос RTSP хаягийн нэвтрэлтийг дарна.
///
/// ffmpeg нь алдааны мөрөндөө хаягийг НУУЦ ҮГТЭЙГЭЭ хэвлэдэг:
/// `Error opening input file rtsp://admin:xxx@192.168.1.100:554/...`
///
/// `nuutsgui` нь зөвхөн бүтэн хаягт зориулсан тул мөр дотор шигтгэгдсэн
/// хаягийг олохгүй. Лог нь дэмжлэгийн хооронд хуулагддаг тул нууц үг
/// тэнд унших боломжтой байх нь болохгүй.
fn mur_nuutsgui(mur: &str) -> String {
    let mut gar = String::with_capacity(mur.len());
    for (i, tasag) in mur.split(' ').enumerate() {
        if i > 0 {
            gar.push(' ');
        }
        gar.push_str(&tasag_nuutsgui(tasag));
    }
    gar
}

/// Нэг үг дотор RTSP хаяг байвал нэвтрэлтийг `***` болгоно.
///
/// СҮҮЛИЙН `@`-аас хойшхийг хост гэж үзнэ — нууц үг дотор `@` байж болно.
fn tasag_nuutsgui(t: &str) -> String {
    for skhem in ["rtsp://", "rtsps://"] {
        if let Some(i) = t.find(skhem) {
            let daraa = &t[i + skhem.len()..];
            if let Some(at) = daraa.rfind('@') {
                return format!("{}{}***@{}", &t[..i], skhem, &daraa[at + 1..]);
            }
        }
    }
    t.to_string()
}

/// Логт нууц үг гаргахгүй.
fn nuutsgui(url: &str) -> String {
    match (url.find("://"), url.find('@')) {
        (Some(a), Some(b)) if b > a + 3 => format!("{}://***@{}", &url[..a], &url[b + 1..]),
        _ => url.to_string(),
    }
}

/// Нэг камерын процессыг мөнхөд харна: унавал дахин асаана.
fn kharuulakh(t: &UrgatsTokhirgoo, yavuulakh: &Arc<AtomicBool>) {
    let shoshgo = &t.shoshgo;
    let mut unasan: u32 = 0;

    while yavuulakh.load(Ordering::SeqCst) {
        let ekhelsen = Instant::now();

        let nevtrelt_aldaa = Arc::new(AtomicBool::new(false));
        match asaaya(&t.ffmpeg, &t.esh, &t.zorilt, shoshgo, t.urgun, &nevtrelt_aldaa) {
            Ok(mut khuukhed) => {
                info!("PUBLISH | {shoshgo} эхэллээ (pid {})", khuukhed.id());

                // Процессыг ӨӨРӨӨ эзэмшиж байгаа тул түүнийг зогсоох эрх
                // энэ урсгалд л байна. `wait()` нь блоклодог учир `try_wait`
                // -ээр байн байн шалгаж, зогсоох дохиог мөн хардаг.
                let buteltsee = loop {
                    if !yavuulakh.load(Ordering::SeqCst) {
                        let _ = khuukhed.kill();
                        let _ = khuukhed.wait();
                        info!("PUBLISH | {shoshgo} зогслоо (үзэгч дууссан)");
                        return;
                    }
                    match khuukhed.try_wait() {
                        Ok(Some(kod)) => break Ok(kod),
                        Ok(None) => std::thread::sleep(Duration::from_millis(500)),
                        Err(e) => break Err(e),
                    }
                };

                let ajillasan = ekhelsen.elapsed().as_secs();
                match buteltsee {
                    Ok(kod) => {
                        warn!("PUBLISH | {shoshgo} зогслоо ({kod}), {ajillasan}s ажилласан")
                    }
                    Err(e) => error!("PUBLISH | {shoshgo} процессыг хүлээхэд алдаа: {e}"),
                }

                if ajillasan >= TOGTVORTOI_SECS {
                    unasan = 0;
                }
            }
            Err(e) => {
                error!(
                    "PUBLISH | {shoshgo} ffmpeg асаах боломжгүй ({}): {e}",
                    t.ffmpeg
                );
            }
        }

        if !yavuulakh.load(Ordering::SeqCst) {
            return;
        }

        unasan = unasan.saturating_add(1);
        // Тогтвортой ажилласны дараах ЭХНИЙ тасалдал нь бараг үргэлж
        // камерын талын түр зуурын зүйл (RTSP сесс таслагдах). Түүнийг
        // удаан хүлээх шалтгаангүй. Дараалан унаж байвал л ухарна.
        let khuleekh = if nevtrelt_aldaa.load(Ordering::SeqCst) {
            warn!(
                "PUBLISH | {shoshgo} НЭВТРЭЛТ БУРУУ (401). Нууц үгийг вэбийн                  камерын тохиргооноос шалга. Төхөөрөмжийн цоож тайлагдах зай                  гаргаж {NEVTRELT_UKHRALT_SECS}s хүлээнэ."
            );
            NEVTRELT_UKHRALT_SECS
        } else if unasan == 1 {
            // Тогтвортой ажилласны дараах ЭХНИЙ тасалдал нь бараг үргэлж
            // камерын талын түр зуурын зүйл (RTSP сесс таслагдах).
            1
        } else {
            std::cmp::min(2u64.saturating_pow(unasan.min(5)), MAX_BACKOFF_SECS)
        };
        if !nevtrelt_aldaa.load(Ordering::SeqCst) {
            warn!("PUBLISH | {shoshgo} {khuleekh}s дараа дахин оролдоно (#{unasan})");
        }

        // Хүлээх зуур зогсоох дохио ирж магадгүй — бүтнээр унтахгүй.
        let khyazgaar = Instant::now() + Duration::from_secs(khuleekh);
        while Instant::now() < khyazgaar {
            if !yavuulakh.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

fn asaaya(
    ffmpeg: &str,
    esh: &str,
    zorilt: &str,
    ip: &str,
    urgun: Option<u32>,
    nevtrelt_aldaa: &Arc<AtomicBool>,
) -> std::io::Result<Child> {
    let mut arg: Vec<String> = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        // UDP дээр пакет алдагдвал зураг задардаг; TCP нь найдвартай.
        "-rtsp_transport".into(),
        "tcp".into(),
        // ТЭМДЭГЛЭЛ: оролтын буферийг хумих тугуудыг (`-fflags nobuffer`,
        // `-flags low_delay`, багасгасан `-analyzeduration`/`-probesize`)
        // ЗӨВИТГӨЖ ХАСАВ. `-c:v copy` үед ffmpeg дүрсийг тайлдаггүй тул тэд
        // саатлыг бараг багасгадаггүй, харин RTSP эх сурвалжийг шинжлэхэд
        // хүрэлцэхгүй болж «source has timed out» үүсгэсэн.
        //
        // `-use_wallclock_as_timestamps`-ыг мөн ХАСАВ: тэр нь камерын өөрийн
        // цагийн тэмдгийг хаяж, пакет ирсэн мөчийн ханын цагаар сольдог.
        // Пакет бөөгнөрөн ирэхэд гарах цагийн шугам тэгш бус болж «фрэйм
        // хооронд гацах» үүсгэдэг.
        "-i".into(),
        esh.into(),
        // Дуу хаяна — WebRTC-д G.711/AAC дамжихгүй.
        "-an".into(),
    ];

    match urgun {
        // Дахин кодчилохгүй: CPU бараг тэг, чанар хэвээр.
        None => {
            arg.push("-c:v".into());
            arg.push("copy".into());
        }
        // Буулгаж кодчилно.
        //
        // Хоёр асуудлыг нэг зэрэг шийднэ:
        //   1. Хэмжээ — 2688×1520 шиг урсгал барилгын upload дээр тогтохгүй
        //   2. Кодек — гаралт нь ҮРГЭЛЖ H.264, иймд H.265 камер ч хөтөч
        //      дээр зурагддаг болно (Chrome нь WebRTC-д HEVC дэмждэггүй)
        Some(u) => {
            // libx264 нь тэгш тоо шаарддаг. Хэт жижиг өгвөл утгагүй.
            let u = u.max(320) & !1u32;
            // Битрэйтийг өргөнөөс гаргана: 1280 → 1280k, 704 → 704k.
            // 15 fps дээр H.264-д хангалттай.
            let kbps = u.max(400);
            arg.extend([
                "-vf".into(),
                // `-2` нь өндрийг харьцаагаар бодож, тэгш тоо болгоно.
                format!("scale={u}:-2"),
                // Хаалганы камерт 21 fps шаардлагагүй — CPU ба зурвас хоёулаа
                // хэмнэгдэнэ. Дугаар таних нь SDK-гаар явдаг тул хамаарахгүй.
                "-r".into(),
                "15".into(),
                "-c:v".into(),
                "libx264".into(),
                // `veryfast` — барилгын PC дээр нэг урсгал кодчилоход тохирно.
                "-preset".into(),
                "veryfast".into(),
                // Кодлогчийн дотоод буферийг хаяна: саатал нэмэхгүй.
                "-tune".into(),
                "zerolatency".into(),
                // `main` — WebRTC дээр хамгийн нийцтэй. `high` нь хөгшин
                // төхөөрөмж дээр гацаж магадгүй.
                "-profile:v".into(),
                "main".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                // 15 fps дээр 2 секунд тутам keyframe — WebRTC нь IDR хүртэл
                // зураг зурдаггүй тул хүйтэн асаалтыг богиносгоно.
                "-g".into(),
                "30".into(),
                "-b:v".into(),
                format!("{kbps}k"),
                "-maxrate".into(),
                format!("{}k", kbps * 5 / 4),
                "-bufsize".into(),
                format!("{}k", kbps * 2),
            ]);
        }
    }

    arg.extend([
        // Mux буфер — анхдагч 0.7с нь урсгал тутамд саатал нэмнэ.
        "-muxdelay".into(),
        "0".into(),
        "-muxpreload".into(),
        "0".into(),
        "-f".into(),
        "rtsp".into(),
        "-rtsp_transport".into(),
        "tcp".into(),
        // RTP пакетын дээд хэмжээ. Анхдагч 1460 нь MediaMTX-ийн 1440-ээс
        // том тул тэр дахин пакетчилдаг (`RTP packets are too big`). 1200 нь
        // интернэтийн MTU-д аюулгүй.
        "-pkt_size".into(),
        "1200".into(),
        zorilt.into(),
    ]);

    let mut khuukhed = Command::new(ffmpeg)
        .args(&arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    // Эцэг зогсоход энэ хүүхэд ч зогсоно — үлдэгдэл процесс үүсэхгүй.
    ajiliin_bag::nemye(&khuukhed);

    // stderr-ийг залгихгүй бол ffmpeg-ийн алдаа хэзээ ч харагдахгүй.
    if let Some(err) = khuukhed.stderr.take() {
        let ip_t = ip.to_string();
        let aldaa_tug = nevtrelt_aldaa.clone();
        std::thread::spawn(move || {
            for mur in BufReader::new(err).lines().map_while(Result::ok) {
                if mur.trim().is_empty() {
                    continue;
                }
                // Нэвтрэлтийн алдааг таньж эцэг урсгалд мэдэгдэнэ — тэр нь
                // ердийн ухралтын оронд урт хүлээлт хэрэглэнэ.
                if mur.contains("401") || mur.contains("Unauthorized") {
                    aldaa_tug.store(true, Ordering::SeqCst);
                }
                warn!("PUBLISH | {ip_t} ffmpeg: {}", mur_nuutsgui(&mur));
            }
        });
    }

    Ok(khuukhed)
}

// ═════════════════════════════════════════════════════════════════════════
//  Серверээс камерын жагсаалт татах
// ═════════════════════════════════════════════════════════════════════════
//
//  ── Яагаад ──────────────────────────────────────────────────────────────
//  Барилга бүрд 8 камер байхад config.toml-д 32 блок гараар бичих нь
//  ядаргаатай, бас өгөгдөл ХОЁР газар (сан ба файл) давхардаж, нууц үг
//  солигдоход хоёуланг нь санах шаардлагатай болно.
//
//  Вэб яг тэр жагсаалтыг `/baiguullaga/:id` -ээс уншдаг: `barilguud[]`
//  дотор `sohCameruud[]`, түүн дотор `ip/port/username/password/root`.
//  Worker-т токен байгаа тул ижил эндпойнтоос өөрөө татаж чадна.
//
//  ЧУХАЛ: RTSP хаягийг вэб талтай ЯГ ижил дүрмээр бүтээнэ — үгүй бол зам
//  зөрж, хөтөч 404 авна. Вэбийн дүрэм (`camera/page.tsx`):
//     ip   = cam.ip       || barilga.cameraIp
//     port = cam.port     || barilga.cameraPort || 554
//     user = cam.username || barilga.cameraUsername
//     pass = cam.password (хуучин "Admin123" бол хоосон) || barilga.cameraPassword
//     root = cam.root     || "stream"
//     url  = user && pass ? rtsp://user:pass@ip:port/root : rtsp://ip:port/root
//
//  Замд зөвхөн IP ба суваг оролцдог тул нэвтрэлт зөрсөн ч зам таарна —
//  нэвтрэлт нь ffmpeg татаж чадах эсэхэд л хамаатай.

/// Серверийн API-ийн суурь хаяг: `…/api/zogsoolSdkService` → `…/api`
fn api_suuri(server_url: &str) -> Option<String> {
    let u = server_url.trim().trim_end_matches('/');
    let i = u.rfind('/')?;
    // "https://" -ийн хоёр налуугаас өмнө хэрчихээс сэргийлнэ
    if i < 9 {
        return None;
    }
    Some(u[..i].to_string())
}

/// JWT-ийн payload-аас `baiguullagiinId`. Тохиргоонд дахин бичүүлэхгүйн тулд.
fn jwt_baiguullaga(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bait = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bait).ok()?;
    v.get("baiguullagiinId")?.as_str().map(|s| s.to_string())
}

/// Хоосон биш мөр авах туслах.
fn mur(v: &serde_json::Value, talbar: &str) -> Option<String> {
    let s = v.get(talbar)?;
    let t = match s {
        serde_json::Value::String(x) => x.trim().to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if t.is_empty() { None } else { Some(t) }
}

/// Хуучин схемийн анхдагч нууц үг — бодит утга биш, хоосонтой адил.
const KHUUCHIN_NUUTS: &str = "Admin123";

/// Вэбийн формоос гараар бичигдсэн утгыг цэвэрлэнэ.
///
/// Бодит өгөгдөлд `cam/realmonitor?channel=1&subtype=1"` гэсэн ТӨГСГӨЛИЙН
/// хашилттай `root` таарсан — хуулж буулгахад орсон нь. Тэр нь RTSP хаягийг
/// эвдэж, оношлоход хүндрэлтэй алдаа болдог. Нууц үгэнд хашилт байж
/// болзошгүй тул үүнийг ЗӨВХӨН зам/хаягийн талбарт хэрэглэнэ.
fn tseverle(v: &str) -> String {
    v.trim()
        // `\u{27}` нь нэг хашилт — escape-ийн ороо цаашид гаргахгүйн тулд.
        .trim_matches(|c: char| c == '"' || c == '\u{27}' || c == ' ')
        .trim_start_matches('/')
        .to_string()
}

fn kameruudiig_olyo(
    cfg: Config,
    pub_cfg: PublishConfig,
    suuri: String,
    mut avsan: HashSet<String>,
) {
    let api = match api_suuri(&cfg.server.url) {
        Some(a) => a,
        None => {
            warn!("PUBLISH | API-ийн суурь хаягийг тодорхойлж чадсангүй — автомат хайлт алгаслаа");
            return;
        }
    };
    let bgid = match jwt_baiguullaga(&cfg.server.token) {
        Some(b) => b,
        None => {
            warn!("PUBLISH | токеноос baiguullagiinId уншиж чадсангүй — автомат хайлт алгаслаа");
            return;
        }
    };

    let url = format!("{api}/baiguullaga/{bgid}");
    info!("PUBLISH | камерын жагсаалт татаж байна: {url}");

    // Сүлжээ хараахан бэлэн болоогүй байж магадгүй — хэд хэдэн удаа оролдоно.
    let mut khariu: Option<serde_json::Value> = None;
    for oroldlogo in 1..=5u32 {
        let res = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .and_then(|c| {
                c.get(&url)
                    .bearer_auth(&cfg.server.token)
                    .send()
            });
        match res {
            Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>() {
                Ok(v) => {
                    khariu = Some(v);
                    break;
                }
                Err(e) => warn!("PUBLISH | жагсаалтыг задлахад алдаа: {e}"),
            },
            Ok(r) => warn!("PUBLISH | жагсаалт татахад HTTP {}", r.status()),
            Err(e) => warn!("PUBLISH | жагсаалт татахад алдаа: {e}"),
        }
        let khuleekh = 5 * oroldlogo as u64;
        warn!("PUBLISH | {khuleekh}s дараа дахин оролдоно (жагсаалт, #{oroldlogo})");
        std::thread::sleep(std::time::Duration::from_secs(khuleekh));
    }

    let khariu = match khariu {
        Some(v) => v,
        None => {
            warn!("PUBLISH | камерын жагсаалт татагдсангүй — зөвхөн config.toml-ынх ажиллана");
            return;
        }
    };

    let barilguud = match khariu.get("barilguud").and_then(|b| b.as_array()) {
        Some(b) => b.clone(),
        None => {
            warn!("PUBLISH | хариунд `barilguud` алга");
            return;
        }
    };

    let mut niit = 0usize;
    for barilga in &barilguud {
        let bid = match mur(barilga, "_id") {
            Some(x) => x,
            None => continue,
        };
        let ner = mur(barilga, "ner").unwrap_or_else(|| bid.clone());

        let kameruud = match barilga.get("sohCameruud").and_then(|c| c.as_array()) {
            Some(c) => c,
            None => continue,
        };

        for cam in kameruud {
            // Вэб дээр унтраасан камерыг нийтлэх шаардлагагүй.
            if cam.get("enabled").and_then(|e| e.as_bool()) == Some(false) {
                continue;
            }

            let ip = match mur(cam, "ip")
                .or_else(|| mur(barilga, "cameraIp"))
                .map(|x| tseverle(&x))
                .filter(|x| !x.is_empty())
            {
                Some(x) => x,
                None => continue,
            };
            let port = mur(cam, "port")
                .or_else(|| mur(barilga, "cameraPort"))
                .map(|x| tseverle(&x))
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| "554".to_string());
            let root = mur(cam, "root")
                .map(|x| tseverle(&x))
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| "stream".to_string());

            let user = mur(cam, "username").or_else(|| mur(barilga, "cameraUsername"));
            let pass = mur(cam, "password")
                .filter(|p| p != KHUUCHIN_NUUTS)
                .or_else(|| mur(barilga, "cameraPassword"));

            let esh = match (user.as_deref(), pass.as_deref()) {
                (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => {
                    let ue = utf8_percent_encode(u, NON_ALPHANUMERIC).to_string();
                    let pe = utf8_percent_encode(p, NON_ALPHANUMERIC).to_string();
                    format!("rtsp://{ue}:{pe}@{ip}:{port}/{root}")
                }
                _ => format!("rtsp://{ip}:{port}/{root}"),
            };

            if pub_cfg.skip_ips.iter().any(|s| s.trim() == ip) {
                continue;
            }

            // Зам нь ЭХНИЙ хаягаас бодогдоно — вэб мөн тэгдэг.
            let dagavar = match crate::config::rtsp_suvag(&esh) {
                Some(s) => format!("-{s}"),
                None => String::new(),
            };
            let zam = format!("{bid}/{}{dagavar}", ip.replace('.', "-"));

            // ffmpeg-ийн эх сурвалжийг л Dahua хэлбэрт хөрвүүлнэ.
            // Хаалганы Dahua төхөөрөмж юу? `[[cameras]]` нь SDK-гаар нэвтрдэг
            // төхөөрөмжүүдийн жагсаалт — бусад нь (СӨХ-ийн NVR) байгаагаараа үлднэ.
            let dahua_esekh = cfg.cameras.iter().any(|c| c.ip.trim() == ip);
            let tatakh = dahua_bolgoyo(&esh, dahua_esekh);
            if tatakh != esh {
                info!(
                    "PUBLISH | {} хөрвүүлэв: {} → {}",
                    zam.rsplit('/').next().unwrap_or(&zam),
                    nuutsgui(&esh),
                    nuutsgui(&tatakh)
                );
            }

            // Автомат олдсон камерууд — ЗӨВХӨН бүртгэнэ, үзэгчээр асна.
            urgats_asaaya(&pub_cfg, &suuri, &mut avsan, &zam, &tatakh, &ner, true, None);
            niit += 1;
        }
    }

    info!(
        "PUBLISH | автомат хайлт дуусав: {} барилга, {niit} камер",
        barilguud.len()
    );
}
