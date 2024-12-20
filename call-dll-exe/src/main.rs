//もしコンソールアプリではなく Windows アプリとしたいなら以下をコメントアウト (MSVC のリンカの場合)
//#![windows_subsystem = "windows"]
use std::env;

use tokio::net::windows::named_pipe::ServerOptions;
use windows::{
    core::*,
    Win32::{
        Foundation::*, Graphics::Gdi::UpdateWindow, System::{LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW}, Threading::{GetStartupInfoW, STARTUPINFOW}}, UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CW_USEDEFAULT, HMENU, MSG, SHOW_WINDOW_CMD, WINDOW_EX_STYLE, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW
        }
    },
};

type TDllTestProc = unsafe extern "cdecl" fn() -> i32;
type TStartHookProc = unsafe extern "cdecl" fn() -> i32;
type TStopHookProc = unsafe extern "cdecl" fn() -> i32;
type TGetPipeName = unsafe extern "cdecl" fn() -> HSTRING;
struct HelloLibraryLoader {
    hdll: HMODULE,
    dll_test_proc: TDllTestProc,
    start_hook_proc: TStartHookProc,
    stop_hook_proc: TStopHookProc,
    get_pipe_name: Option<TGetPipeName>,
}


#[tokio::main]
async fn main() -> Result<()> {
    // wWinMain 相当の情報を取得
    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    let args: Vec<String> = env::args().collect();
    let n_show_cmd = {
        let mut si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        unsafe {
            GetStartupInfoW(&mut si);
        };
        si.wShowWindow as i32
    };
    
    // DLL ロード
    let dll = HelloLibraryLoader::new()?;

    // パイプでメッセージを受け取るスレッドの開始
    if let Some(get_pipe_name) = dll.get_pipe_name {
        let pipe_name = unsafe { get_pipe_name() };
        start_message_pipe_server(&pipe_name.to_string());
    }

    // DLL start hook
    unsafe {
        assert_eq!((dll.dll_test_proc)(), 334);
        assert_eq!((dll.start_hook_proc)(), 0);
    }

    // ウィンドウクラス
    // sample: https://github.com/microsoft/windows-rs/blob/master/crates/samples/windows/create_window/src/main.rs
    let lpclassname: PCWSTR = w!("MainMenuClass");
    let mut wc = WNDCLASSW::default();
    wc.lpfnWndProc = Some(window_proc);
    wc.hInstance = hinstance;
    wc.lpszMenuName = w!("MainMenu");
    wc.lpszClassName = lpclassname;
    let atom = unsafe { RegisterClassW(&wc) };
    assert_ne!(atom, 0);
    let hwnd_main = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            lpclassname,
            w!("Main"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            HWND(core::ptr::null_mut()),
            HMENU(core::ptr::null_mut()),
            hinstance,
            Option::None
        )?
    };
    unsafe {
        //ShowWindow(hwndMain, SHOW_WINDOW_CMD(nShowCmd));
        let _ = ShowWindow(hwnd_main, SHOW_WINDOW_CMD(5));
        let _ = UpdateWindow(hwnd_main);
    };

    // メッセージループ
    let mut msg = MSG::default();
    let mut b_ret;
    while {
        b_ret = unsafe { GetMessageW(&mut msg, HWND(core::ptr::null_mut()), 0, 0) };
        BOOL(0) != b_ret
    } {
        if b_ret == BOOL(-1) {
            continue;
        } else {
            unsafe {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
    }

    // stop hook
    unsafe {
        _ = (dll.stop_hook_proc)();
    }

    return Ok(());
}

unsafe extern "system" fn window_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM
) -> LRESULT {
    match msg {
        WM_DESTROY => PostQuitMessage(0),
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    };

    return LRESULT(0);
}

// IPC で文字列を受け取り、標準出力に書き出すスレッドの作成
fn start_message_pipe_server(pipe_name: &str) {
    let pipe_name_copy = String::from(pipe_name);
    tokio::spawn(async move {
        let mut pipe_server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name_copy).unwrap();
        // 名前付きパイプサーバーループを開始
        // ref: https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/struct.NamedPipeServer.html
        _ = tokio::spawn(async move {
            loop {
                pipe_server.connect().await.unwrap();
                let connected_client = pipe_server;
                pipe_server = ServerOptions::new().create(&pipe_name_copy).unwrap();
                _ = tokio::spawn(async move {
                    // client action
                    loop {
                        tokio::task::yield_now().await;
                        if let Err(_err) = connected_client.readable().await {
                            return; // exit
                        }
                        let mut buf= [0u8; 8192];
                        match connected_client.try_read(&mut buf) {
                            Ok(0) => continue,
                            Ok(n) => {
                                let vec: Vec<u8> = buf.to_vec();
                                let zero_pos = vec.iter().position(|&i| i == 0);
                                let line = String::from_utf8_lossy(&vec[..(zero_pos.unwrap_or(n))]).into_owned();
                                println!("{}", line);
                                if zero_pos.is_some() { return; } // \0 が含まれていたら終了。
                            },
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => continue,
                            Err(_err) => continue,
                        };                          
                    }
                });
            }
        });
    });
}

// https://github.com/microsoft/windows-rs/blob/master/crates/samples/windows/delay_load/src/main.rs
impl HelloLibraryLoader {
    pub fn new() -> Result<Self> {
        let hdll = unsafe { LoadLibraryW(w!(r".\hook_dll.dll"))? };
        if hdll.is_invalid() {
            return Err(Error::from_win32());
        }
        let dll_test_proc = Self::get_proc::<TDllTestProc>(hdll, s!("DllTest"))?;
        let start_hook_proc = Self::get_proc::<TStartHookProc>(hdll, s!("StartHook"))?;
        let stop_hook_proc = Self::get_proc::<TStopHookProc>(hdll, s!("StopHook"))?;

        // GetPipeName は Option
        let get_pipe_name = if let Ok(proc) = Self::get_proc::<TGetPipeName>(hdll, s!("GetPipeName")) {
            Some(proc)
        } else {
            None
        };

        Ok(Self { hdll, dll_test_proc, start_hook_proc, stop_hook_proc, get_pipe_name })
    }

    fn get_proc<T>(hdll: HMODULE, lpprocname: PCSTR) -> Result<T> {
        unsafe {
            let address = match GetProcAddress(hdll, lpprocname) {
                None => return Err(Error::from_win32()),
                Some(v) => v,
            };
            Ok(std::mem::transmute_copy(&address))
        }
    }
}

impl Drop for HelloLibraryLoader {
    fn drop(&mut self) {
        if !self.hdll.is_invalid() {
            _ = unsafe { FreeLibrary(self.hdll) };
            self.hdll = HMODULE::default();
        }
    }
}
