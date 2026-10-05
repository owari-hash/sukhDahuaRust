//! FFI bindings to dhnetsdk.dll (Dahua NetSDK)
//! 64-bit only â€” all pointers are 8 bytes

#![allow(non_snake_case, dead_code, non_camel_case_types)]

use std::ffi::{c_int, c_uint, c_void, c_char};
use libloading::{Library, Symbol};
use once_cell::sync::OnceCell;

pub type BOOL   = c_int;
pub type DWORD  = c_uint;
pub type HANDLE = *mut c_void;
pub type HWND   = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct NET_DEVICEINFO_Ex {
    pub sSerialNumber:        [u8; 48],
    pub byAlarmInPortNum:     u8,
    pub byAlarmOutPortNum:    u8,
    pub byDiskNum:            u8,
    pub byDVRType:            u8,
    pub byChanNum:            u8,
    pub byStartChan:          u8,
    pub byAudioChanNum:       u8,
    pub byIPChanNum:          u8,
    pub byZeroChanNum:        u8,
    pub byMainProto:          u8,
    pub bySubProto:           u8,
    pub bySupport:            u8,
    pub bySupport2:           u8,
    pub bySupport3:           u8,
    pub byMultiServer:        u8,
    pub byLimitLoginTime:     u8,
    pub byLimitLoginInterval: u8,
    pub reserve:              [u8; 238],
}

impl Default for NET_DEVICEINFO_Ex {
    fn default() -> Self { unsafe { std::mem::zeroed() } }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct NET_CTRL_OPEN_STROBE {
    pub dwSize:        DWORD,
    pub nChannelId:    c_int,
    pub szPlateNumber: [u8; 32],
    pub reserve:       [u8; 124],
}

impl Default for NET_CTRL_OPEN_STROBE {
    fn default() -> Self {
        let mut s: Self = unsafe { std::mem::zeroed() };
        s.dwSize = std::mem::size_of::<Self>() as DWORD;
        s
    }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct NET_IN_LOGIN_WITH_HIGHLEVEL_SECURITY {
    pub dwSize:     DWORD,
    pub szIP:       [u8; 64],
    pub nPort:      c_int,
    pub szUserName: [u8; 64],
    pub szPassword: [u8; 64],
    pub emSpecCap:  c_int,
    pub pCapParam:  *mut c_void,
}

impl Default for NET_IN_LOGIN_WITH_HIGHLEVEL_SECURITY {
    fn default() -> Self {
        let mut s: Self = unsafe { std::mem::zeroed() };
        s.dwSize = std::mem::size_of::<Self>() as DWORD;
        s
    }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct NET_OUT_LOGIN_WITH_HIGHLEVEL_SECURITY {
    pub dwSize:     DWORD,
    pub stuDevInfo: NET_DEVICEINFO_Ex,
}

impl Default for NET_OUT_LOGIN_WITH_HIGHLEVEL_SECURITY {
    fn default() -> Self {
        let mut s: Self = unsafe { std::mem::zeroed() };
        s.dwSize = std::mem::size_of::<Self>() as DWORD;
        s
    }
}

pub const EM_CTRL_OPEN_STROBE: c_int = 263;
pub const CFG_CMD_TRAFFIC_LATTICE_SCREEN: c_int = 10000; // CLIENT_SetConfig type ID for TrafficLatticeScreen

// â”€â”€â”€ Screen display â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
pub const EM_SCREEN_SHOW_CONTENTS_CUSTOM:            c_int = 0;
pub const EM_SCREEN_SHOW_CONTENTS_INTIME:            c_int = 5;
pub const EM_SCREEN_SHOW_CONTENTS_OUTTIME:           c_int = 6;
pub const EM_LATTICE_SCREEN_SHOW_TYPE_WORD_CONTROL:  c_int = 1;
pub const EM_LATTICE_SCREEN_CONTROL_TYPE_CAMERA_CONTROL: c_int = 1;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct NET_TRAFFIC_LATTICE_SCREEN_SHOW_CONTENTS {
    pub emContents:  c_int,
    pub szCustomStr: [u8; 128],
    pub byReserved1: [u8; 4],
    pub byReserved:  [u8; 32],
}

impl Default for NET_TRAFFIC_LATTICE_SCREEN_SHOW_CONTENTS {
    fn default() -> Self { unsafe { std::mem::zeroed() } }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct NET_TRAFFIC_LATTICE_SCREEN_INFO {
    pub nContentsNum: c_int,
    pub stuContents:  [NET_TRAFFIC_LATTICE_SCREEN_SHOW_CONTENTS; 64],
}

impl Default for NET_TRAFFIC_LATTICE_SCREEN_INFO {
    fn default() -> Self { unsafe { std::mem::zeroed() } }
}

#[repr(C)]
#[derive(Clone)]
pub struct NET_CFG_TRAFFIC_LATTICE_SCREEN_INFO {
    pub dwSize:             DWORD,
    pub nStatusChangeTime:  c_int,
    // NOTE: stuNormal and stuCarPass come BEFORE emShowType/emControlType (C# SDK order)
    pub stuNormal:          NET_TRAFFIC_LATTICE_SCREEN_INFO,
    pub stuCarPass:         NET_TRAFFIC_LATTICE_SCREEN_INFO,
    pub emShowType:         c_int,
    pub emControlType:      c_int,
    pub emBackgroundMode:   c_int,
    pub szPlayList:         [u8; 640],  // 10 * 64 chars
    pub nPlayListNum:       c_int,
    pub stuLogoInfo:        [u8; 512],  // NET_TRAFFIC_LATTICE_SCREEN_LOGO_INFO placeholder
    pub stuAlarmNoticeInfo: [u8; 512],  // NET_TRAFFIC_LATTICE_SCREEN_ALARM_NOTICE_INFO placeholder
}

impl Default for NET_CFG_TRAFFIC_LATTICE_SCREEN_INFO {
    fn default() -> Self {
        let mut s: Self = unsafe { std::mem::zeroed() };
        s.dwSize = std::mem::size_of::<Self>() as DWORD;
        s
    }
}

// â”€â”€â”€ Callbacks â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
pub type fDisConnectCallBack =
    unsafe extern "system" fn(lLoginID: HANDLE, pchDVRIP: *const c_char, nDVRPort: c_int, dwUser: *mut c_void);

pub type fHaveReConnectCallBack =
    unsafe extern "system" fn(lLoginID: HANDLE, pchDVRIP: *const c_char, nDVRPort: c_int, dwUser: *mut c_void);

// â”€â”€â”€ DahuaSdk â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
static SDK: OnceCell<DahuaSdk> = OnceCell::new();

pub struct DahuaSdk {
    _lib: Library,

    pub init_ex:          unsafe extern "system" fn(cbDisConnect: Option<fDisConnectCallBack>, dwUser: *mut c_void, lpInitParam: *mut c_void) -> BOOL,
    pub cleanup:          unsafe extern "system" fn(),
    pub login_ex2:        unsafe extern "system" fn(pstInParam: *const NET_IN_LOGIN_WITH_HIGHLEVEL_SECURITY, pstOutParam: *mut NET_OUT_LOGIN_WITH_HIGHLEVEL_SECURITY, nWaitTime: c_int) -> HANDLE,
    pub logout:           unsafe extern "system" fn(lLoginID: HANDLE) -> BOOL,
    pub control_device:   unsafe extern "system" fn(lLoginID: HANDLE, emType: c_int, pInBuf: *mut c_void, nWaitTime: c_int) -> BOOL,
    pub get_last_error:   unsafe extern "system" fn() -> DWORD,
    pub set_reconnect:    unsafe extern "system" fn(cbAutoConnect: Option<fHaveReConnectCallBack>, dwUser: *mut c_void),
    pub set_connect_time: unsafe extern "system" fn(nWaitTime: c_int, nTrytimes: c_int),
    pub set_dev_config:   unsafe extern "system" fn(lLoginID: HANDLE, szCommand: *const c_char, nChannelID: c_int, lpInBuffer: *mut c_void, dwInBufferSize: DWORD, pReserved: *mut c_void, nWaitTime: c_int) -> BOOL,
    pub set_config:       unsafe extern "system" fn(lLoginID: HANDLE, nCfgType: c_int, nChannelID: c_int, lpInBuffer: *mut c_void, dwInBufferSize: DWORD, nWaitTime: c_int, pReserved1: *mut c_void, pReserved2: *mut c_void) -> BOOL,
    pub set_traffic_lattice_screen: Option<unsafe extern "system" fn(
    lLoginID: HANDLE, 
    pstInfo: *mut NET_CFG_TRAFFIC_LATTICE_SCREEN_INFO,
    nWaitTime: c_int,
) -> BOOL>,
    pub make_key_frame: Option<unsafe extern "system" fn(lLoginID: HANDLE, nChannelID: c_int, nSubStream: c_int) -> BOOL>,
}

unsafe impl Send for DahuaSdk {}
unsafe impl Sync for DahuaSdk {}

impl DahuaSdk {
    pub fn load() -> anyhow::Result<&'static Self> {
        SDK.get_or_try_init(|| unsafe {
            let exe_dir = std::env::current_exe()?
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_owned();
            let dll_path = exe_dir.join("dhnetsdk.dll");

            let lib = Library::new(&dll_path)
                .map_err(|e| anyhow::anyhow!("Cannot load {}: {e}", dll_path.display()))?;

            macro_rules! sym {
                ($name:literal, $T:ty) => {{
                    let s: Symbol<$T> = lib.get($name)
                        .map_err(|e| anyhow::anyhow!("Symbol {:?} not found: {e}", $name))?;
                    *s
                }};
            }

             Ok(DahuaSdk {
                init_ex:          sym!(b"CLIENT_InitEx\0",                        unsafe extern "system" fn(Option<fDisConnectCallBack>, *mut c_void, *mut c_void) -> BOOL),
                cleanup:          sym!(b"CLIENT_Cleanup\0",                       unsafe extern "system" fn()),
                login_ex2:        sym!(b"CLIENT_LoginWithHighLevelSecurity\0",     unsafe extern "system" fn(*const NET_IN_LOGIN_WITH_HIGHLEVEL_SECURITY, *mut NET_OUT_LOGIN_WITH_HIGHLEVEL_SECURITY, c_int) -> HANDLE),
                logout:           sym!(b"CLIENT_Logout\0",                         unsafe extern "system" fn(HANDLE) -> BOOL),
                control_device:   sym!(b"CLIENT_ControlDevice\0",                  unsafe extern "system" fn(HANDLE, c_int, *mut c_void, c_int) -> BOOL),
                get_last_error:   sym!(b"CLIENT_GetLastError\0",                   unsafe extern "system" fn() -> DWORD),
                set_reconnect:    sym!(b"CLIENT_SetAutoReconnect\0",               unsafe extern "system" fn(Option<fHaveReConnectCallBack>, *mut c_void)),
                set_connect_time: sym!(b"CLIENT_SetConnectTime\0",                 unsafe extern "system" fn(c_int, c_int)),
                set_dev_config:   sym!(b"CLIENT_SetDevConfig\0",                   unsafe extern "system" fn(HANDLE, *const c_char, c_int, *mut c_void, DWORD, *mut c_void, c_int) -> BOOL),
                set_config:       sym!(b"CLIENT_SetConfig\0",                       unsafe extern "system" fn(HANDLE, c_int, c_int, *mut c_void, DWORD, c_int, *mut c_void, *mut c_void) -> BOOL),
                set_traffic_lattice_screen: {
                        let s: Result<Symbol<unsafe extern "system" fn(HANDLE, *mut NET_CFG_TRAFFIC_LATTICE_SCREEN_INFO, c_int) -> BOOL>, _> 
                                    = lib.get(b"CLIENT_SetTrafficLatticeScreen\0");
                                        s.ok().map(|f| *f)},
                make_key_frame: {
                    let s: Result<Symbol<unsafe extern "system" fn(HANDLE, c_int, c_int) -> BOOL>, _> = lib.get(b"CLIENT_MakeKeyFrame\0");
                    s.ok().map(|f| *f)
                },
                _lib: lib, 
            })
        })
    }
}

// â”€â”€â”€ Helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
pub fn fill_ansi(dst: &mut [u8], src: &str) {
    dst.fill(0);
    let bytes = src.as_bytes();
    let len = bytes.len().min(dst.len().saturating_sub(1));
    dst[..len].copy_from_slice(&bytes[..len]);
}

