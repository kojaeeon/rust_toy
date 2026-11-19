//! Hotkeys 실행 파일의 진입점. 빌드 대상 OS에 따라 다른 `main`을 컴파일한다.

// Windows 일반 실행에서 콘솔 창 없이 트레이 앱으로 시작한다. 테스트 빌드에는 적용하지 않는다.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

// TOML 파서는 OS 독립적이다. Windows 실행과 모든 OS의 테스트에서 컴파일한다.
#[cfg(any(windows, test))]
mod config;
// Win32 FFI를 사용하는 모듈은 Windows 대상으로 빌드할 때만 포함한다.
#[cfg(windows)]
mod windows_app;

#[cfg(windows)]
fn main() {
    // 정상 종료에는 팝업이 없다. 시작이나 메시지 루프에서 전달된 오류만 보여 준다.
    if let Err(error) = windows_app::run() {
        windows_app::show_error(&error);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Hotkeys runs on Windows.");
}
