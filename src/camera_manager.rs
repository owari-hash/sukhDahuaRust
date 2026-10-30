use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use once_cell::sync::Lazy;
use std::ffi::{c_void, c_char, c_int};
use log::{warn, error};
use once_cell::sync::OnceCell;
use tokio::sync::mpsc;

use crate::config::{CameraEntry, Config, SdkConfig};
use crate::sdk::{
    DahuaSdk, NET_IN_LOGIN_WITH_HIGHLEVEL_SECURITY, NET_OUT_LOGIN_WITH_HIGHLEVEL_SECURITY,
    NET_CTRL_OPEN_STROBE, EM_CTRL_OPEN_STROBE,
    fill_ansi, HANDLE,
};

static LAST_OPEN: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));
pub static CAMERA_MANAGER: OnceCell<CameraManager> = OnceCell::new();

#[derive(Debug, Clone)]
pub struct PlateEvent {
    pub plate:     String,
    pub camera_ip: String,
}

#[derive(Clone)]
struct CameraState {
    handle:   HANDLE,
    ip:       String,
    password: String,
}

unsafe impl Send for CameraState {}
unsafe impl Sync for CameraState {}

pub struct CameraManager {
    handle_map:     Arc<Mutex<HashMap<String, HANDLE>>>,
    cameras:        Arc<Mutex<Vec<CameraState>>>,
    reconnect_lock: Arc<Mutex<()>>,
    sdk_cfg:        SdkConfig,
    cam_cfg:        Vec<CameraEntry>,
    pub plate_tx:   mpsc::Sender<PlateEvent>,
}

unsafe impl Send for CameraManager {}
unsafe impl Sync for CameraManager {}


unsafe extern "system" fn on_disconnect(
    login_id: HANDLE,
    pch_dvr_ip: *const c_char,
    port: c_int,
    _dw_user: *mut c_void,
) {
    let ip = if !pch_dvr_ip.is_null() {
        std::ffi::CStr::from_ptr(pch_dvr_ip).to_string_lossy().into_owned()
    } else {
        "unknown".to_string()
    };
    log::warn!("SDK CALLBACK | Камер холболт тасарлаа: ip={ip}, port={port}, handle={login_id:?}");
    if let Some(mgr) = CAMERA_MANAGER.get() {
        mgr.mark_disconnected(&ip);
    }
}

unsafe extern "system" fn on_reconnect(
    login_id: HANDLE,
    pch_dvr_ip: *const c_char,
    port: c_int,
    _dw_user: *mut c_void,
) {
    let ip = if !pch_dvr_ip.is_null() {
        std::ffi::CStr::from_ptr(pch_dvr_ip).to_string_lossy().into_owned()
    } else {
        "unknown".to_string()
    };
    log::info!("SDK CALLBACK | Камер NetSDK-ээр амжилттай дахин холбогдлоо: ip={ip}, port={port}, handle={login_id:?}");
    if let Some(mgr) = CAMERA_MANAGER.get() {
        mgr.update_handle(&ip, login_id);
    }
}

impl CameraManager {
    pub fn new(cfg: &Config, plate_tx: mpsc::Sender<PlateEvent>) -> Self {
        Self {
            handle_map:     Arc::new(Mutex::new(HashMap::new())),
            cameras:        Arc::new(Mutex::new(Vec::new())),
            reconnect_lock: Arc::new(Mutex::new(())),
            sdk_cfg:        cfg.sdk.clone(),
            cam_cfg:        cfg.cameras.clone(),
            plate_tx,
        }
    }

    pub fn mark_disconnected(&self, ip: &str) {
        if let Ok(mut map) = self.handle_map.lock() {
            map.remove(ip);
        }
        if let Ok(mut cams) = self.cameras.lock() {
            if let Some(cam) = cams.iter_mut().find(|c| c.ip == ip) {
                cam.handle = std::ptr::null_mut();
            }
        }
    }

    pub fn update_handle(&self, ip: &str, handle: HANDLE) {
        if handle.is_null() {
            return;
        }
        if let Ok(mut map) = self.handle_map.lock() {
            map.insert(ip.to_string(), handle);
        }
        if let Ok(mut cams) = self.cameras.lock() {
            if let Some(cam) = cams.iter_mut().find(|c| c.ip == ip) {
                cam.handle = handle;
            } else {
                let password = self.password_for_ip(ip);
                cams.push(CameraState {
                    handle,
                    ip: ip.to_string(),
                    password,
                });
            }
        }
    }

    pub fn handle_for_ip(&self, ip: &str) -> Option<HANDLE> {
        self.handle_map.lock().ok()?.get(ip).copied()
    }

    pub fn make_key_frame(&self, ip: &str, channel: i32, substream: i32) -> bool {
        if let Some(handle) = self.handle_for_ip(ip) {
            if let Ok(sdk) = DahuaSdk::load() {
                if let Some(mkf) = sdk.make_key_frame {
                    let ok = unsafe { mkf(handle, channel, substream) };
                    return ok != 0;
                }
            }
        }
        false
    }

    pub fn camera_count(&self) -> usize {
        self.handle_map.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_entrance(&self, ip: &str) -> bool {
        self.cam_cfg
            .iter()
            .find(|c| c.ip == ip)
            .map(|c| c.gate.as_deref().unwrap_or("").to_lowercase() == "entrance")
            .unwrap_or(false)
    }

    pub fn org_name(&self) -> &str {
        &self.sdk_cfg.org_name
    }

    pub fn company_name(&self) -> &str {
        &self.sdk_cfg.company_name
    }

    pub fn password_for_ip(&self, ip: &str) -> String {
        self.cam_cfg
            .iter()
            .find(|c| c.ip == ip)
            .map(|c| c.password.clone())
            .unwrap_or_else(|| "admin123".to_string())
    }

    pub fn http_port_for_ip(&self, ip: &str) -> Option<u16> {
        self.cam_cfg
            .iter()
            .find(|c| c.ip == ip)
            .and_then(|c| c.http_port)
    }


    pub fn sambar_type_for_ip(&self, ip: &str) -> String {
        self.cam_cfg
            .iter()
            .find(|c| c.ip == ip)
            .and_then(|c| c.sambar_type.clone())
            .unwrap_or_else(|| "sambar".to_string())
    }

    pub fn startup_and_connect(&self) -> anyhow::Result<()> {
        let sdk = DahuaSdk::load()?;

        log::info!("SDK | initialize хийж байна...");
        let ret = unsafe { (sdk.init_ex)(Some(on_disconnect), std::ptr::null_mut(), std::ptr::null_mut()) };
        if ret == 0 {
            anyhow::bail!("CLIENT_InitEx failed");
        }
        log::info!("SDK | амжилттай аслаа");

        unsafe {
            (sdk.set_connect_time)(
                self.sdk_cfg.connect_timeout_ms as i32,
                self.sdk_cfg.max_connect_retries as i32,
            );
            (sdk.set_reconnect)(Some(on_reconnect), std::ptr::null_mut());
        }

        self.connect_all();
        Ok(())
    }

    pub fn connect_all(&self) {
        let sdk = match DahuaSdk::load() { Ok(s) => s, Err(e) => { error!("{e}"); return; } };

        let mut map  = self.handle_map.lock().unwrap();
        let mut cams = self.cameras.lock().unwrap();
        map.clear();
        cams.clear();

        for cam in &self.cam_cfg {
            log::info!("SDK | camera Ñ…Ð¾Ð»Ð±Ð¾Ð¶ Ð±Ð°Ð¹Ð½Ð°: {}", cam.ip);

            let handle = Self::connect_with_retry_inner(sdk, &cam.ip, &cam.password, &self.sdk_cfg);
            if handle.is_null() {
                log::error!("SDK | camera Ñ…Ð¾Ð»Ð±Ð¾Ð³Ð´ÑÐ¾Ð½Ð³Ò¯Ð¹: {}", cam.ip);
                continue;
            }

            log::info!("SDK | camera Ð°Ð¼Ð¶Ð¸Ð»Ñ‚Ñ‚Ð°Ð¹ Ñ…Ð¾Ð»Ð±Ð¾Ð³Ð´Ð»Ð¾Ð¾: {}", cam.ip);
            map.insert(cam.ip.clone(), handle);
            cams.push(CameraState {
                handle,
                ip:       cam.ip.clone(),
                password: cam.password.clone(),
            });
        }
    }

    fn connect_with_retry_inner(sdk: &DahuaSdk, ip: &str, password: &str, sdk_cfg: &SdkConfig) -> HANDLE {
        let max  = sdk_cfg.max_connect_retries as usize;
        let port = sdk_cfg.port as i32;

        for attempt in 1..=max {
            println!("Ð¥Ð¾Ð»Ð±Ð¾Ð»Ñ‚ Ð¾Ñ€Ð¾Ð»Ð´Ð»Ð¾Ð³Ð¾ {attempt}/{max} â€” {ip}");

            let mut in_param  = NET_IN_LOGIN_WITH_HIGHLEVEL_SECURITY::default();
            let mut out_param = NET_OUT_LOGIN_WITH_HIGHLEVEL_SECURITY::default();

            fill_ansi(&mut in_param.szIP,       ip);
            fill_ansi(&mut in_param.szUserName, &sdk_cfg.username);
            fill_ansi(&mut in_param.szPassword, password);
            in_param.nPort     = port;
            in_param.emSpecCap = 0; // TCP

            let handle = unsafe {
                (sdk.login_ex2)(&in_param, &mut out_param, sdk_cfg.connect_timeout_ms as i32)
            };

            if !handle.is_null() {
                // Төхөөрөмжийн чадамжийг НЭВТРЭХ үед л мэдэж болно. `out_param`-ыг
                // өмнө нь хаяж байсан тул "яагаад энэ камер дээр хаалга
                // нээгдэхгүй вэ" гэдгийг логоос тогтоох арга байсангүй.
                //
                // `alarmOut=0` бол тухайн төхөөрөмжид реле гаралт алга —
                // CLIENT_ControlDevice(OPEN_STROBE) заавал унана. Ажиллаж
                // байгаа камертай зэрэгцүүлж харахад ялгаа нь шууд харагдана.
                let dev = &out_param.stuDevInfo;
                let serial: String = dev
                    .sSerialNumber
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| c as char)
                    .collect();
                log::info!(
                    "SDK DEV | ip={ip} serial={serial} devType={} chan={} alarmOut={} alarmIn={}",
                    dev.byDVRType,
                    dev.byChanNum,
                    dev.byAlarmOutPortNum,
                    dev.byAlarmInPortNum,
                );
                println!("ÐÐ¼Ð¶Ð¸Ð»Ñ‚Ñ‚Ð°Ð¹ Ñ…Ð¾Ð»Ð±Ð¾Ð³Ð´Ð»Ð¾Ð¾: {ip} (attempt {attempt})");
                return handle;
            }

            let err = unsafe { (sdk.get_last_error)() };
            warn!("Ð¥Ð¾Ð»Ð±Ð¾Ð»Ñ‚ Ð°Ð¼Ð¶Ð¸Ð»Ñ‚Ð³Ò¯Ð¹ {ip} attempt {attempt}: error {err:#x}");

            if attempt < max {
                let delay_ms = 1000u64 * attempt as u64;
                println!("{}ms Ñ…Ò¯Ð»ÑÑÐ¶ Ð±Ð°Ð¹Ð½Ð°...", delay_ms);
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
        }
        std::ptr::null_mut()
    }

    /// Attempts to open the strobe/gate for `ip`. Returns the real SDK error
    /// code on failure (instead of a bare bool) so the caller can report the
    /// actual reason upstream - this is what ends up in the `execute-open-result`
    /// ack sent back to the server, which is otherwise the only place that
    /// failure detail would ever be visible.
    pub fn open_gate(&self, ip: &str, plate: Option<&str>) -> Result<(), String> {
        // Хэрэв сүүлийн 2.5 секундэд энэ IP дээр хаалт нээх комманд амжилттай
        // өгөгдсөн бол шлагбаум аль хэдийн сөхөгдөж байгаа тул давхар импульсийг алгасна.
        if let Ok(map) = LAST_OPEN.lock() {
            if let Some(last) = map.get(ip) {
                if last.elapsed() < std::time::Duration::from_millis(2500) {
                    log::info!("GATE DEBOUNCE | ip={ip} аль хэдийн нээгдэж байна (давхар коммандыг алгаслаа)");
                    return Ok(());
                }
            }
        }

        let sdk = match DahuaSdk::load() { Ok(s) => s, Err(e) => return Err(format!("sdk_load_failed: {e}")) };
        let handle = match self.handle_for_ip(ip) {
            Some(h) => h,
            None => { warn!("open_gate: IP {ip} handle Ð¾Ð»Ð´ÑÐ¾Ð½Ð³Ò¯Ð¹"); return Err("no_handle".to_string()); }
        };

        let mut strobe = NET_CTRL_OPEN_STROBE::default();
        strobe.nChannelId = 0;

        let ret = unsafe {
            (sdk.control_device)(
                handle,
                EM_CTRL_OPEN_STROBE,
                &mut strobe as *mut NET_CTRL_OPEN_STROBE as *mut c_void,
                5000,
            )
        };
        if ret != 0 {
            if let Ok(mut map) = LAST_OPEN.lock() {
                map.insert(ip.to_string(), Instant::now());
            }
            return Ok(());
        }

        let err = unsafe { (sdk.get_last_error)() };
        log::error!("GATE FAIL  | ip={ip} err={err:#x} â€” reconnecting");

        // Handle Ñ…ÑƒÑƒÑ‡Ð¸Ñ€ÑÐ°Ð½ Ð±Ð¾Ð» Ð´Ð°Ñ…Ð¸Ð½ Ñ…Ð¾Ð»Ð±Ð¾Ð³Ð´Ð¾Ð½Ð¾
        self.reconnect_single(ip);

        let new_handle = match self.handle_for_ip(ip) {
            Some(h) => h,
            None => {
                log::error!("GATE RETRY FAIL | ip={ip} â€” no handle after reconnect");
                return Err(format!("no_handle_after_reconnect (first_err={err:#x})"));
            }
        };

        let mut strobe2 = NET_CTRL_OPEN_STROBE::default();
        strobe2.nChannelId = 0;
        let ret2 = unsafe {
            (sdk.control_device)(
                new_handle,
                EM_CTRL_OPEN_STROBE,
                &mut strobe2 as *mut NET_CTRL_OPEN_STROBE as *mut c_void,
                5000,
            )
        };
        if ret2 != 0 {
            if let Ok(mut map) = LAST_OPEN.lock() {
                map.insert(ip.to_string(), Instant::now());
            }
            return Ok(());
        }

        let err2 = unsafe { (sdk.get_last_error)() };
        log::error!("GATE RETRY FAIL | ip={ip} err={err2:#x}");

        // ГУРАВ дахь оролдлого — улсын дугаартай.
        //
        // Зарим ITC төхөөрөмж "шаанг гараар нээх"-ийг хүлээж авдаггүй бөгөөд
        // зөвхөн тодорхой дугаарт зөвшөөрөл олгох хэлбэрээр нээдэг.
        // `szPlateNumber` талбар нь бүтцэд нь аль хэдийн байсан ч хоосон
        // явуулдаг байв — сервер дугаарыг илгээдэг мөртлөө энд хаягддаг байсан.
        //
        // Эхний хоёр оролдлогыг ХЭВЭЭР үлдээв: өнөөдөр ажиллаж байгаа
        // камеруудын зан төлөв огт өөрчлөгдөхгүй, зөвхөн АЛЬ ХЭДИЙН унасан
        // тохиолдолд л энэ нэмэлт оролдлого явна.
        if let Some(p) = plate.filter(|p| !p.is_empty()) {
            let mut strobe3 = NET_CTRL_OPEN_STROBE::default();
            strobe3.nChannelId = 0;
            fill_ansi(&mut strobe3.szPlateNumber, p);

            let ret3 = unsafe {
                (sdk.control_device)(
                    new_handle,
                    EM_CTRL_OPEN_STROBE,
                    &mut strobe3 as *mut NET_CTRL_OPEN_STROBE as *mut c_void,
                    5000,
                )
            };
            if ret3 != 0 {
                if let Ok(mut map) = LAST_OPEN.lock() {
                    map.insert(ip.to_string(), Instant::now());
                }
                log::warn!("GATE OK (plate) | ip={ip} plate={p} — хоосон дугаараар нээгдээгүй, дугаартайгаар нээгдлээ");
                return Ok(());
            }

            let err3 = unsafe { (sdk.get_last_error)() };
            log::error!("GATE PLATE FAIL | ip={ip} plate={p} err={err3:#x}");
            return Err(format!(
                "sdk_err={err2:#x} (first_err={err:#x}, plate_err={err3:#x})"
            ));
        }

        Err(format!("sdk_err={err2:#x} (first_err={err:#x})"))
    }

    fn reconnect_single(&self, ip: &str) {
        let _guard = self.reconnect_lock.lock().unwrap();

        let sdk = match DahuaSdk::load() { Ok(s) => s, Err(_) => return };
        let password = self.password_for_ip(ip);

        println!("Дахин холбогдож байна: {ip}");

        // Хуучин handle logout хийх
        if let Some(old_handle) = self.handle_for_ip(ip) {
            unsafe { let _ = (sdk.logout)(old_handle); }
        }

        let handle = Self::connect_with_retry_inner(sdk, ip, &password, &self.sdk_cfg);
        if !handle.is_null() {
            self.update_handle(ip, handle);
            println!("Дахин амжилттай холбогдлоо: {ip}");
        } else {
            self.mark_disconnected(ip);
            error!("Дахин холбогдоход амжилтгүй: {ip}");
        }
    }

    pub fn check_sdk_connections(&self) {
        let ips_to_check: Vec<String> = {
            self.cam_cfg.iter().map(|c| c.ip.clone()).collect()
        };

        let sdk_port = self.sdk_cfg.port;

        for ip in ips_to_check {
            let handle = self.handle_for_ip(&ip);
            let has_valid_handle = match handle {
                Some(h) => !h.is_null(),
                None => false,
            };

            let addr = format!("{ip}:{sdk_port}");
            let reachable = match addr.parse::<std::net::SocketAddr>() {
                Ok(sock_addr) => std::net::TcpStream::connect_timeout(
                    &sock_addr,
                    std::time::Duration::from_millis(1500),
                ).is_ok(),
                Err(_) => false,
            };

            if !reachable {
                if has_valid_handle {
                    log::warn!("SDK HEARTBEAT | {ip} сүлжээнд холбогдохгүй байна (unreachable)");
                    self.mark_disconnected(&ip);
                }
            } else {
                if !has_valid_handle {
                    log::info!("SDK HEARTBEAT | {ip} сүлжээнд орж ирлээ, SDK холболт сэргээж байна...");
                    self.reconnect_single(&ip);
                }
            }
        }
    }

    pub fn reconnect_all(&self) {
        println!("Ð‘Ò¯Ñ… ÐºÐ°Ð¼ÐµÑ€Ñ‹Ð³ Ð´Ð°Ñ…Ð¸Ð½ Ñ…Ð¾Ð»Ð±Ð¾Ð¶ Ð±Ð°Ð¹Ð½Ð°...");
        self.connect_all();
    }
}

