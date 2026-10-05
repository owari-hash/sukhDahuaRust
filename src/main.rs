//! dahua-service.exe â€” Rust port of C# Dahua camera parking service
//!
//! Usage:
//!   dahua-service.exe install    â€” install as Windows service
//!   dahua-service.exe uninstall  â€” remove Windows service
//!   dahua-service.exe run        â€” run interactively (debug)
//!   (no args)                    â€” called by Windows SCM

mod sdk;
mod config;
mod camera_manager;
mod plate_listener;
mod plate_service;
mod api;
mod socket_bridge;
mod probe;
mod publisher;

use std::ffi::OsString;
use std::time::Duration;
use log::{info, error, LevelFilter};
use tokio::sync::mpsc;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode,
        ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
    service::{ServiceAccess, ServiceStartType, ServiceErrorControl, ServiceInfo},
};

use camera_manager::{CameraManager, CAMERA_MANAGER};
use plate_service::PlateService;
use config::Config;

const VERSION:         &str = env!("CARGO_PKG_VERSION");
const SERVICE_NAME:    &str = "DahuaParkingService";
const SERVICE_DISPLAY: &str = "zevDahuaRust";
const SERVICE_DESC:    &str = "Dahua ALPR camera plate reader with barrier control";

// â”€â”€â”€ Windows Service boilerplate â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

define_windows_service!(ffi_service_main, service_main);

fn service_main(args: Vec<OsString>) {
    if let Err(e) = run_service(args) {
        error!("Service fatal error: {e}");
    }
}

fn run_service(_args: Vec<OsString>) -> anyhow::Result<()> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

    let event_handler = move |control: ServiceControl| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint:        0,
        wait_hint:         Duration::from_secs(15),
        process_id:        None,
    })?;

    init_logging(LevelFilter::Info);

    let cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            error!("Config load failed: {e}");
            status_handle.set_service_status(ServiceStatus {
                service_type:      ServiceType::OWN_PROCESS,
                current_state:     ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code:         ServiceExitCode::ServiceSpecific(1),
                checkpoint: 0, wait_hint: Duration::ZERO, process_id: None,
            })?;
            return Ok(());
        }
    };

    status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint: 0, wait_hint: Duration::ZERO, process_id: None,
    })?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    rt.block_on(async {
        tokio::select! {
            result = run_app(cfg) => {
                if let Err(e) = result { error!("App error: {e}"); }
            }
            _ = tokio::task::spawn_blocking(move || { let _ = stop_rx.recv(); }) => {
                info!("Stop signal received");
            }
        }
    });

    status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint: 0, wait_hint: Duration::ZERO, process_id: None,
    })?;

    Ok(())
}


async fn run_app(cfg: Config) -> anyhow::Result<()> {
    info!("=== Dahua Parking Service {VERSION} starting ===");

    // Suppress debug error dialogs from DLLs
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(
            windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX
        );
    }

    if cfg.sambar_only {
        info!("sambar_only mode skipping SDK/camera, running API only");

        // Still need CameraManager for password/port/is_entrance lookups in sambar endpoints
        let (plate_tx, _plate_rx) = mpsc::channel::<camera_manager::PlateEvent>(1);
        let manager = CameraManager::new(&cfg, plate_tx);
        CAMERA_MANAGER.set(manager)
            .map_err(|_| anyhow::anyhow!("CameraManager already initialized"))?;

        api::run_api_server(5000).await;
        return Ok(());
    }

    // 1. Plate event channel
    let (plate_tx, mut plate_rx) = mpsc::channel::<camera_manager::PlateEvent>(128);

    // 2. Init CameraManager
    let manager = CameraManager::new(&cfg, plate_tx.clone());
    CAMERA_MANAGER.set(manager)
        .map_err(|_| anyhow::anyhow!("CameraManager already initialized"))?;

    // 3. PlateService
    let plate_svc = std::sync::Arc::new(PlateService::new(cfg.server.clone())?);

    // 4. Connect cameras (blocking)
    tokio::task::spawn_blocking(move || {
        if let Err(e) = CAMERA_MANAGER.get().unwrap().startup_and_connect() {
            error!("SDK startup failed: {e}");
        }
    });

    // 4b. Камерын урсгалыг VPS рүү тасралтгүй нийтлэх.
    //
    // Хөтөч ЭНЭ компьютер руу P2P холбогдохоо болино: нийтийн сервер
    // дээрээс татна. Камер руу орох RTSP сесс үзэгчийн тооноос үл хамаарч
    // камер тус бүрт НЭГ л болно.
    publisher::start(&cfg);

    // 5. Start HTTP plate listeners for each camera
    for cam in &cfg.cameras {
        let ip       = cam.ip.clone();
        let password = cam.password.clone();
        let port = cam.http_port.unwrap_or(443);
        let tx       = plate_tx.clone();
        tokio::spawn(async move {
            plate_listener::run_plate_listener(ip, password, port, tx).await;
        });
    }

    // 6. SDK heartbeat â€” checks gate connection every 60s, reconnects if dead
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = tokio::task::spawn_blocking(|| {
                if let Some(mgr) = CAMERA_MANAGER.get() {
                    mgr.check_sdk_connections();
                }
            }).await;
        }
    });

    // 7. API server
    tokio::spawn(api::run_api_server(5000));

    // 7b. Socket.io bridge to VPS (amarhome.mn) for all configured buildings
    //
    // АЛЬ ХАЯГ Dahua төхөөрөмж рүү чиглэсэнийг bridge-д дуулгана:
    // зөвхөн тэдний RTSP замыг Dahua хэлбэрт болгоно. СӨХ-ийн
    // Hikvision NVR байгаагаараа үлдвэл зөв.
    socket_bridge::dahua_ipuud_tavya(cfg.cameras.iter().map(|c| {
        // Хоосон зам = «тохиргоонд заагаагүй» → bridge хуучинчлан
        // Hikvision→Dahua хөрвүүлэлтээ хийнэ.
        let zam = c
            .root
            .as_deref()
            .map(str::trim)
            .map(|r| r.trim_start_matches('/'))
            .unwrap_or("")
            .to_string();
        (c.ip.clone(), zam)
    }));

    let vps_url = "https://amarhome.mn".to_string();
    let targets = cfg.targets();
    info!("Bridging {} building(s) for WebRTC stream sharing", targets.len());
    for target in targets {
        info!("   - {} ({})", target.label, target.barilgiin_id);
        socket_bridge::start_socket_bridge(vps_url.clone(), target);
    }

    // 8. Plate event processor
    info!("Plate event processor started");
    loop {
        match plate_rx.recv().await {
            Some(event) => {
                let svc = std::sync::Arc::clone(&plate_svc);
                tokio::spawn(async move {
                    svc.process_plate(&event).await;
                });
            }
            None => {
                println!("Plate channel closed, keeping service alive...");
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }
        }
    }
}

// â”€â”€â”€ Entry point â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn main() {
    // Suppress debug error dialogs
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(
            windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX
        );
    }

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("install")   => install_service(),
        Some("uninstall") => uninstall_service(),
        Some("run")       => run_interactive(),
        // Without this there is no way to tell which build a machine is
        // actually running, so a rollout can never be verified.
        Some("--version") | Some("-v") => println!("{SERVICE_DISPLAY} {VERSION}"),
        _ => {
            service_dispatcher::start(SERVICE_NAME, ffi_service_main)
                .expect("Service dispatcher failed â€” run as Windows service or use 'run'");
        }
    }
}

fn run_interactive() {
    init_logging(LevelFilter::Debug);
    println!("Running interactively (not as Windows service)");

    let cfg = Config::load().expect("Cannot load config.toml");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            if let Err(e) = run_app(cfg).await {
                error!("Error: {e}");
            }
        });
}

// â”€â”€â”€ Service install / uninstall â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn install_service() {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    ).expect("Open SCM failed â€” run as Administrator");

    let exe = std::env::current_exe().expect("Cannot get exe path");
    let info = ServiceInfo {
        name:             OsString::from(SERVICE_NAME),
        display_name:     OsString::from(SERVICE_DISPLAY),
        service_type:     ServiceType::OWN_PROCESS,
        start_type:       ServiceStartType::AutoStart,
        error_control:    ServiceErrorControl::Normal,
        executable_path:  exe,
        launch_arguments: vec![],
        dependencies:     vec![],
        account_name:     None,
        account_password: None,
    };

    match manager.create_service(&info, ServiceAccess::CHANGE_CONFIG) {
        Ok(svc) => {
            svc.set_description(SERVICE_DESC).ok();

            // Configure auto-restart on failure: restart after 5s, 10s, 30s; reset counter after 60s
            let _ = std::process::Command::new("sc")
                .args([
                    "failure", SERVICE_NAME,
                    "reset=", "60",
                    "actions=", "restart/5000/restart/10000/restart/30000",
                ])
                .output();

            println!("âœ“ Service '{SERVICE_NAME}' installed.");
            println!("  Auto-restart on failure: 5s / 10s / 30s");
            println!("  Start: net start {SERVICE_NAME}");
        }
        Err(e) => { eprintln!("âœ— Install failed: {e}"); std::process::exit(1); }
    }
}

fn uninstall_service() {
    let manager = ServiceManager::local_computer(
        None::<&str>, ServiceManagerAccess::CONNECT,
    ).expect("Open SCM failed");

    let svc = manager.open_service(
        SERVICE_NAME, ServiceAccess::DELETE | ServiceAccess::STOP,
    ).expect("Service not found");

    let _ = svc.stop();
    std::thread::sleep(Duration::from_secs(2));

    match svc.delete() {
        Ok(_)  => println!("âœ“ Service '{SERVICE_NAME}' removed."),
        Err(e) => { eprintln!("âœ— Uninstall failed: {e}"); std::process::exit(1); }
    }
}

fn init_logging(level: LevelFilter) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let log_path = exe_dir.join("service.log");

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}] [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                message
            ))
        })
        .level(level)
        // HTTP-client internals (connection pooling, header parsing, byte counts)
        // drown out our own lines at Debug level and say nothing useful about
        // gate/stream behaviour â€” keep them at Warn regardless of our level.
        .level_for("hyper", LevelFilter::Warn)
        .level_for("hyper_util", LevelFilter::Warn)
        .level_for("reqwest", LevelFilter::Warn)
        .level_for("rustls", LevelFilter::Warn)
        .level_for("native_tls", LevelFilter::Warn)
        .level_for("tokio_util", LevelFilter::Warn)
        .level_for("want", LevelFilter::Warn)
        .level_for("mio", LevelFilter::Warn)
        .level_for("h2", LevelFilter::Warn)
        .chain(std::io::stdout())
        .chain(fern::log_file(&log_path).expect("Cannot open log file"))
        .apply()
        .expect("Logger init failed");

    log::info!("Log file: {}", log_path.display());
}
