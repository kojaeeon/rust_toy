//! 숨은 Win32 창, 트레이 아이콘, 전역 단축키와 입력 전송을 관리한다.

use crate::config::{self, Action, Binding, ALT, CTRL, SHIFT, WIN};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// 트레이 콜백은 앱 전용 메시지를 받는다. 타이머 ID와 메뉴 항목 ID는 각 API 안에서 구분된다.
const TRAY_MESSAGE: u32 = WM_APP + 1;
const SEND_TIMER: usize = 1;
const EDIT: usize = 1;
const EXIT: usize = 2;
const MODIFIERS: [(u32, u16); 4] = [
    (CTRL, VK_CONTROL),
    (ALT, VK_MENU),
    (SHIFT, VK_SHIFT),
    (WIN, VK_LWIN),
];

// 키를 뗄 때까지 기다리는 한 건의 입력. 처음 활성화되어 있던 창과 시작 시각을 기억한다.
struct Pending {
    index: usize,
    target: HWND,
    started: Instant,
}

// 창 프로시저가 참조하는 상태. run_window가 실행되는 동안 호출자의 스택에 살아 있다.
struct State {
    bindings: Vec<Binding>,
    config_path: PathBuf,
    pending: Option<Pending>,
    taskbar_created: u32,
}

// W 접미사 Win32 API에 전달할 UTF-16 문자열. 끝의 0은 C 문자열의 종료 표시다.
fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error(operation: &str) -> String {
    format!("{operation}: {}", io::Error::last_os_error())
}

// 메시지 박스 호출이 끝날 때까지 임시 Vec이 살아 있어 전달한 포인터를 읽을 수 있다.
pub fn show_error(text: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(text).as_ptr(),
            wide("Hotkeys").as_ptr(),
            MB_ICONERROR,
        );
    }
}

/// 실행 파일 옆에서 설정을 읽은 뒤 Win32 창과 메시지 루프를 시작한다.
pub fn run() -> Result<(), String> {
    let mut config_path = std::env::current_exe().map_err(|error| error.to_string())?;
    config_path.set_file_name("hotkeys.toml");
    let source = fs::read_to_string(&config_path)
        .map_err(|error| format!("Cannot read {}: {error}", config_path.display()))?;
    let bindings = config::parse(&source)?;
    let mut state = State {
        bindings,
        config_path,
        pending: None,
        taskbar_created: 0,
    };
    unsafe { run_window(&mut state) }
}

// Win32가 요구하는 콜백 시그니처와 구조체 초기화 규약을 따른다.
// GWLP_USERDATA의 포인터는 state가 살아 있는 동안만 연결하며 창을 파괴하기 전에 지운다.
unsafe fn run_window(state: &mut State) -> Result<(), String> {
    let instance = GetModuleHandleW(null());
    let class_name = wide("MoveHotkeysWindow");
    let mut class: WNDCLASSW = zeroed();
    class.hInstance = instance;
    class.lpszClassName = class_name.as_ptr();
    class.lpfnWndProc = Some(window_proc);
    if RegisterClassW(&class) == 0 {
        return Err(last_error("Cannot register the tray window"));
    }
    // 숨은 최상위 창도 Explorer가 보내는 TaskbarCreated 브로드캐스트를 받을 수 있다.
    let window = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide("Hotkeys").as_ptr(),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if window.is_null() {
        return Err(last_error("Cannot create the tray window"));
    }

    // 등록 개수를 별도로 추적한다. 클로저 안에서 오류로 반환해도 바깥에서 성공분을 정리한다.
    let mut registered = 0;
    let result = (|| {
        for binding in &state.bindings {
            let id = registered + 1;
            if RegisterHotKey(
                window,
                id,
                // 키를 계속 누를 때 반복 WM_HOTKEY가 발생하는 것을 막는다.
                binding.chord.modifiers | MOD_NOREPEAT,
                u32::from(binding.chord.key),
            ) == 0
            {
                return Err(last_error(&format!("Cannot register {}", binding.name)));
            }
            registered = id;
        }
        state.taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        // Win32 콜백에 Rust 상태를 넘기는 연결점. 수명 관리는 Rust 쪽에서 책임진다.
        SetWindowLongPtrW(window, GWLP_USERDATA, state as *mut State as isize);
        if !tray_icon(window, NIM_ADD) {
            return Err("Cannot add the notification area icon.".into());
        }
        let mut message: MSG = zeroed();
        loop {
            // GetMessageW: 양수는 처리할 메시지, 0은 WM_QUIT, -1은 오류다.
            let status = GetMessageW(&mut message, null_mut(), 0, 0);
            if status == 0 {
                return Ok(());
            }
            if status == -1 {
                return Err(last_error("Cannot read Windows messages"));
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    })();

    // 정상 종료, 등록 중 실패, 메시지 루프 실패가 이 정리 경로를 공유한다.
    for id in 1..=registered {
        UnregisterHotKey(window, id);
    }
    SetWindowLongPtrW(window, GWLP_USERDATA, 0);
    tray_icon(window, NIM_DELETE);
    DestroyWindow(window);
    result
}

unsafe fn tray_icon(window: HWND, operation: u32) -> bool {
    // 셸은 cbSize로 구조체 크기를 확인하고 창 핸들과 아이콘 ID의 쌍으로 아이콘을 식별한다.
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = 1;
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
    let tip = wide("Hotkeys");
    data.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(operation, &data) != 0
}

// extern "system"은 운영체제 콜백 호출 규약을 선택한다.
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
    // 창 생성·종료 중에는 상태 포인터가 비어 있을 수 있다. 기본 창 프로시저에 위임한다.
    let Some(state) = state.as_mut() else {
        return DefWindowProcW(window, message, wparam, lparam);
    };
    match message {
        WM_HOTKEY => {
            // 등록 ID는 1부터, Vec 인덱스는 0부터다. 잘못된 ID는 범위 검사로 걸러낸다.
            if let Some(index) = wparam
                .checked_sub(1)
                .filter(|id| *id < state.bindings.len())
            {
                let target = GetForegroundWindow();
                // 콜백을 막지 않고 20ms 타이머로 키 해제를 확인한다. 새 입력은 대기 슬롯을 덮어쓴다.
                if !target.is_null() && SetTimer(window, SEND_TIMER, 20, None) != 0 {
                    state.pending = Some(Pending {
                        index,
                        target,
                        started: Instant::now(),
                    });
                }
            }
        }
        WM_TIMER if wparam == SEND_TIMER => {
            if let Some(pending) = state.pending.as_ref() {
                let binding = &state.bindings[pending.index];
                // 다른 창으로 전환됐거나 3초가 지나면 예약을 버린다.
                if GetForegroundWindow() != pending.target
                    || pending.started.elapsed() > Duration::from_secs(3)
                {
                    state.pending = None;
                    KillTimer(window, SEND_TIMER);
                } else if keys_released(binding.chord.key) {
                    // 물리 키가 모두 해제된 뒤 전송해야 사용자의 보조키 상태와 섞이지 않는다.
                    state.pending = None;
                    KillTimer(window, SEND_TIMER);
                    if !send_action(&binding.action) {
                        MessageBeep(MB_ICONWARNING);
                    }
                }
            }
        }
        TRAY_MESSAGE if lparam as u32 == WM_RBUTTONUP || lparam as u32 == WM_CONTEXTMENU => {
            // 메뉴 안의 중첩 메시지 루프가 콜백을 다시 부를 수 있어 경로를 소유 값으로 넘긴다.
            let config_path = state.config_path.clone();
            show_tray_menu(window, config_path);
        }
        WM_CLOSE => PostQuitMessage(0),
        // Explorer가 다시 시작되면 트레이 아이콘만 다시 추가한다.
        other if other != 0 && other == state.taskbar_created => {
            tray_icon(window, NIM_ADD);
        }
        _ => return DefWindowProcW(window, message, wparam, lparam),
    }
    0
}

unsafe fn show_tray_menu(window: HWND, config_path: PathBuf) {
    // 메뉴 핸들은 직접 해제한다. 선택 ID를 반환받아 설정 열기와 종료를 분기한다.
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    AppendMenuW(menu, MF_STRING, EDIT, wide("Open settings").as_ptr());
    AppendMenuW(menu, MF_STRING, EXIT, wide("Exit").as_ptr());
    let mut point: POINT = zeroed();
    if GetCursorPos(&mut point) != 0 {
        SetForegroundWindow(window);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            null(),
        );
        PostMessageW(window, WM_NULL, 0, 0);
        match selected as usize {
            EDIT => {
                let result = ShellExecuteW(
                    window,
                    wide("open").as_ptr(),
                    wide(config_path).as_ptr(),
                    null(),
                    null(),
                    SW_SHOWNORMAL,
                );
                if result as isize <= 32 {
                    show_error("Open hotkeys.toml in a text editor, then restart Hotkeys.");
                }
            }
            EXIT => PostQuitMessage(0),
            _ => {}
        }
    }
    DestroyMenu(menu);
}

// GetAsyncKeyState 반환값의 최상위 비트가 설정되면 현재 눌린 키다.
// 부호 있는 반환값이 음수인지 확인해 이 비트를 판별한다.
unsafe fn keys_released(key: u16) -> bool {
    [key, VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        .all(|key| GetAsyncKeyState(i32::from(key)) >= 0)
}

// INPUT의 태그를 키보드로 지정하고 유니언의 ki 필드에 키 이벤트를 채운다.
fn input(key: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

unsafe fn send_action(action: &Action) -> bool {
    // 입력 배열을 먼저 모두 구성한 뒤 순서대로 SendInput에 한 번 전달한다.
    let mut events = Vec::new();
    match action {
        Action::Text(text) => {
            // 키보드 배열을 거치지 않고 UTF-16 코드 유닛을 보낸다. 이모지는 두 유닛이 될 수 있다.
            for unit in text.encode_utf16() {
                events.push(input(0, unit, KEYEVENTF_UNICODE));
                events.push(input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
            }
        }
        Action::Send(chord) => {
            // 보조키 누름 → 주키 누름/해제 → 보조키 역순 해제. 반복자를 복제해 양방향으로 순회한다.
            let active = MODIFIERS
                .into_iter()
                .filter(|(bit, _)| chord.modifiers & bit != 0);
            for (_, key) in active.clone() {
                events.push(input(key, 0, key_flags(key)));
            }
            let flags = key_flags(chord.key);
            events.push(input(chord.key, 0, flags));
            events.push(input(chord.key, 0, flags | KEYEVENTF_KEYUP));
            for (_, key) in active.rev() {
                events.push(input(key, 0, key_flags(key) | KEYEVENTF_KEYUP));
            }
        }
    }
    // 반환값은 실제로 큐에 추가된 이벤트 수다. 일부만 추가되어도 실패로 판단한다.
    SendInput(
        events.len() as u32,
        events.as_ptr(),
        size_of::<INPUT>() as i32,
    ) == events.len() as u32
}

// 방향키 등 Win32가 확장 키로 분류하는 가상 키에는 별도 플래그를 붙인다.
fn key_flags(key: u16) -> u32 {
    match key {
        VK_PRIOR | VK_NEXT | VK_END | VK_HOME | VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN
        | VK_INSERT | VK_DELETE | VK_LWIN => KEYEVENTF_EXTENDEDKEY,
        _ => 0,
    }
}
