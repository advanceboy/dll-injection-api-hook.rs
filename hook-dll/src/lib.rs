use std::{mem, sync::{Mutex, OnceLock}};
use windows::{
    core::*,
    Win32::{
        Foundation::*, Storage::FileSystem::{CreateFileW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_WRITE, OPEN_EXISTING}, System::{LibraryLoader::{GetModuleFileNameW, GetModuleHandleW}, Pipes::WaitNamedPipeW, SystemServices::*, Threading::GetCurrentProcessId}, UI::WindowsAndMessaging as wm, UI::WindowsAndMessaging::*
    },
};

const PIPE_NAME: &str = r"\\.\pipe\named-pipe-for-dll-hook-message";

struct HInstanceWrapper(HINSTANCE);
struct HHookWrapper(HHOOK);
unsafe impl Send for HInstanceWrapper {}
unsafe impl Send for HHookWrapper {}

static HINSTANCE_GLOBAL: Mutex<Option<HInstanceWrapper>> = Mutex::new(None);
static HMOUSEHOOK_GLOBAL: Mutex<Option<HHookWrapper>> = Mutex::new(None);
static HWNDHOOK_GLOBAL: Mutex<Option<HHookWrapper>> = Mutex::new(None);
static FILENAME_GLOBAL: OnceLock<String> = OnceLock::<String>::new();

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(dll_module: HINSTANCE, call_reason: u32, _: *mut ()) -> bool {
    match call_reason {
        DLL_PROCESS_ATTACH => attach(dll_module),
        DLL_PROCESS_DETACH => detach(),
        _ => ()
    };
    return true;
}

#[no_mangle]
extern "C" fn DllTest() -> i32 { 334 }

#[no_mangle]
extern "C" fn GetPipeName() -> HSTRING { HSTRING::from(PIPE_NAME) }

#[no_mangle]
extern "C" fn StartHook() -> i32 {
    // フック開始
    let h_instance = {
        match HINSTANCE_GLOBAL.try_lock() {
            Err(_) => return 1,
            Ok(option) => {
                match option.as_ref() {
                    None => return 1,
                    Some(h_instance) => h_instance.0,
                }
            }
        }
    };

    match (HMOUSEHOOK_GLOBAL.try_lock(), HWNDHOOK_GLOBAL.try_lock()) {
        (Ok(mut h_mouse_option), Ok(mut h_wnd_option)) => {
            if let Ok(hook) = unsafe { SetWindowsHookExW(WH_MOUSE, Some(hook_mouse_proc), h_instance, 0) } {
                *h_mouse_option = Some(HHookWrapper(hook));
            }
            if let Ok(hook) = unsafe { SetWindowsHookExW(WH_CALLWNDPROC, Some(hook_wnd_proc), h_instance, 0) } {
                *h_wnd_option = Some(HHookWrapper(hook));
            }
        },
        _ => return 1,
    }

    return 0;
}

#[no_mangle]
extern "C" fn StopHook() -> i32 {
    match (|| -> ::core::result::Result<(), Box<dyn ::core::error::Error>> {
        for mtx in [&HMOUSEHOOK_GLOBAL, &HWNDHOOK_GLOBAL] {
            let option = mtx.try_lock()?;
            match option.as_ref() {
                None => (),
                Some(hhook) => {
                    if let Err(err) = unsafe { UnhookWindowsHookEx(hhook.0) } {
                        eprintln!("{}", err);
                        return Err(Box::new(err));
                    }
                }
            }
        }
        Ok(())
    })() {
        Err(_) => return 1,
        Ok(()) => return 0,
    }
}

fn write_main_console(line: &str) {
    let mut counter = 0;
    let hpipe = loop {
        match unsafe {
            CreateFileW(
                &HSTRING::from(PIPE_NAME),
                GENERIC_WRITE.0,
                FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES::default(), // FILE_FLAG_OVERLAPPED
                HANDLE::default()
            )
        } {
            Ok(h) => break h,
            Err(e) if e.code() == HRESULT::from_win32(ERROR_PIPE_BUSY.0) => (),
            Err(_e) => return,
        }
        if counter >= 10 { return }
        counter += 1;
        if !unsafe { WaitNamedPipeW(&HSTRING::from(PIPE_NAME), 10) }.as_bool() {
            return;
        }
    };
    
    let mut line_t = line.as_bytes().to_vec();
    line_t.push(0); // 終了を示すために末尾に \0 をつける
    _ = unsafe { WriteFile(
        hpipe,
        Some(&line_t),
        None,
        None
    ) };
    _ = unsafe { CloseHandle(hpipe) };
}

fn attach(dll_module: HINSTANCE) {
    // DLL の hInstance をグローバル変数に設定
    if let Ok(mut mtx) = HINSTANCE_GLOBAL.try_lock() {
        *mtx = Some(HInstanceWrapper(dll_module));
    }

    // モジュール名の取得
    let mut text_buff = [0u16; 512];
    let pid = unsafe { GetCurrentProcessId() };
    let len = unsafe{ GetModuleFileNameW(GetModuleHandleW(PCWSTR::null()).unwrap(), &mut text_buff) };
    let loaded_message = if len > 0 {
        let module_path = String::from_utf16(&text_buff[0..len as usize]).unwrap();
        if let Some(file_name) = std::path::Path::new(&module_path).file_name() {
            _ = FILENAME_GLOBAL.set(file_name.to_string_lossy().into_owned());
        }
        format!("We've loaded the library. as instance {} (pid: {}) from \"{}\"", dll_module.0 as u64, pid, module_path)
    } else {
        format!("We've loaded the library. as instance {} (pid: {})", dll_module.0 as u64, pid)
    };
    write_main_console(&loaded_message); // StartHook 呼ぶ前は書き込まれない
}

fn detach() { }

fn parse_mouse_msg(msg: WPARAM) -> Option<String> {
    let result = match msg.0 as usize as u32 {
        wm::WM_APPCOMMAND => "WM_APPCOMMAND", wm::WM_CAPTURECHANGED => "WM_CAPTURECHANGED", wm::WM_LBUTTONDBLCLK => "WM_LBUTTONDBLCLK", wm::WM_LBUTTONDOWN => "WM_LBUTTONDOWN", wm::WM_LBUTTONUP => "WM_LBUTTONUP", wm::WM_MBUTTONDBLCLK => "WM_MBUTTONDBLCLK", wm::WM_MBUTTONDOWN => "WM_MBUTTONDOWN", wm::WM_MBUTTONUP => "WM_MBUTTONUP", wm::WM_MOUSEACTIVATE => "WM_MOUSEACTIVATE", wm::WM_MOUSEHWHEEL => "WM_MOUSEHWHEEL", wm::WM_MOUSEMOVE => "WM_MOUSEMOVE", wm::WM_MOUSEWHEEL => "WM_MOUSEWHEEL", wm::WM_NCHITTEST => "WM_NCHITTEST", wm::WM_NCLBUTTONDBLCLK => "WM_NCLBUTTONDBLCLK", wm::WM_NCLBUTTONDOWN => "WM_NCLBUTTONDOWN", wm::WM_NCLBUTTONUP => "WM_NCLBUTTONUP", wm::WM_NCMBUTTONDBLCLK => "WM_NCMBUTTONDBLCLK", wm::WM_NCMBUTTONDOWN => "WM_NCMBUTTONDOWN", wm::WM_NCMBUTTONUP => "WM_NCMBUTTONUP", wm::WM_NCMOUSEHOVER => "WM_NCMOUSEHOVER", wm::WM_NCMOUSELEAVE => "WM_NCMOUSELEAVE", wm::WM_NCMOUSEMOVE => "WM_NCMOUSEMOVE", wm::WM_NCRBUTTONDBLCLK => "WM_NCRBUTTONDBLCLK", wm::WM_NCRBUTTONDOWN => "WM_NCRBUTTONDOWN", wm::WM_NCRBUTTONUP => "WM_NCRBUTTONUP", wm::WM_NCXBUTTONDBLCLK => "WM_NCXBUTTONDBLCLK", wm::WM_NCXBUTTONDOWN => "WM_NCXBUTTONDOWN", wm::WM_NCXBUTTONUP => "WM_NCXBUTTONUP", wm::WM_RBUTTONDBLCLK => "WM_RBUTTONDBLCLK", wm::WM_RBUTTONDOWN => "WM_RBUTTONDOWN", wm::WM_RBUTTONUP => "WM_RBUTTONUP", wm::WM_XBUTTONDBLCLK => "WM_XBUTTONDBLCLK", wm::WM_XBUTTONDOWN => "WM_XBUTTONDOWN", wm::WM_XBUTTONUP => "WM_XBUTTONUP",
        _ => return None,
    };
    return Some(result.to_string());
}

unsafe extern "system" fn hook_mouse_proc(
    ncode: i32, wparam: WPARAM, lparam: LPARAM
) -> LRESULT {
    if ncode >= 0 {
        let prefix = match FILENAME_GLOBAL.get() {
            Some(string) => format!("{}: ", string),
            None => String::new(),
        };
        let mmsg = mem::transmute::<LPARAM, &MOUSEHOOKSTRUCTEX>(lparam);
        let msg_display = if let Some(message_str) = parse_mouse_msg(wparam) {
            format!("{}: {}", message_str, wparam.0)
        } else {
            format!("{}", wparam.0)
        };
        match wparam.0 as usize as u32 {
            wm::WM_MOUSEWHEEL | wm::WM_MOUSEHWHEEL => {
                write_main_console(&format!("{}mouse msg ({}): x/y = {}/{}, wheel: {}", prefix, msg_display, mmsg.Base.pt.x, mmsg.Base.pt.y, mmsg.mouseData as i32 >> 16));
            },
            wm::WM_XBUTTONDOWN | wm::WM_XBUTTONUP | wm::WM_XBUTTONDBLCLK | wm::WM_NCXBUTTONDOWN | wm::WM_NCXBUTTONUP | wm::WM_NCXBUTTONDBLCLK => {
                write_main_console(&format!("{}mouse msg ({}): x/y = {}/{}, button target: {}", prefix, msg_display, mmsg.Base.pt.x, mmsg.Base.pt.y, mmsg.mouseData >> 16 & 0xffff));
            },
            _ => {
                write_main_console(&format!("{}mouse msg ({}): x/y = {}/{}", prefix, msg_display, mmsg.Base.pt.x, mmsg.Base.pt.y));
            }
        }
        
    }
    return CallNextHookEx(None, ncode, wparam, lparam);
}

unsafe extern "system" fn hook_wnd_proc(
    ncode: i32, wparam: WPARAM, lparam: LPARAM
) -> LRESULT {
    if ncode >= 0 {
        let prefix = match FILENAME_GLOBAL.get() {
            Some(string) => format!("{}: ", string),
            None => String::new(),
        };
        let mcwp = mem::transmute::<LPARAM, &CWPSTRUCT>(lparam);
        match mcwp.message {
            wm::WM_CLOSE => {
                write_main_console(&format!("{}WM_CLOSE of hWnd({}): wParam/lParam = {}/{}", prefix, mcwp.hwnd.0 as usize, mcwp.wParam.0, mcwp.lParam.0));
            },
            wm::WM_MOVING => {
                write_main_console(&format!("{}WM_MOVING of hWnd({}): wParam/lParam = {}/{}", prefix, mcwp.hwnd.0 as usize, mcwp.wParam.0, mcwp.lParam.0));
            },
            _ => (),
        };
    }

    return CallNextHookEx(None, ncode, wparam, lparam);
}
