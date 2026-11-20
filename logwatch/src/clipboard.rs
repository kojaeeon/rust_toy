use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub(crate) fn copy(text: &str) -> Result<()> {
    let mut command = clipboard_command();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "clipboard command unavailable: {}",
                command.get_program().display()
            )
        })?;
    // 로그는 명령 인수가 아닌 표준 입력으로 전달한다. 자식이 종료되기 전에 파이프를 닫는다.
    let write = child.stdin.take().unwrap().write_all(text.as_bytes());
    let status = child.wait()?;
    ensure!(status.success(), "clipboard command exited with {status}");
    write.context("could not send text to clipboard")
}

#[cfg(windows)]
fn clipboard_command() -> Command {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let mut command = Command::new(
        std::path::PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
    );
    command.args([
        "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
        "[Console]::InputEncoding = [Text.UTF8Encoding]::new($false); Set-Clipboard -Value ([Console]::In.ReadToEnd()) -ErrorAction Stop",
    ]);
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes to the Windows clipboard; run in a test session"]
    fn windows_clipboard_round_trip_preserves_unicode_and_lines() {
        let text = "<Dataset>\n  <Col>한글 &amp; 값</Col>\n</Dataset>";
        copy(text).unwrap();

        let output = Command::new(clipboard_command().get_program())
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::Write((Get-Clipboard -Raw))",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let pasted = String::from_utf8(output.stdout).unwrap();
        assert_eq!(pasted.replace("\r\n", "\n"), text);
    }
}

#[cfg(target_os = "macos")]
fn clipboard_command() -> Command {
    Command::new("pbcopy")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn clipboard_command() -> Command {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Command::new("wl-copy")
    } else {
        let mut command = Command::new("xclip");
        command.args(["-selection", "clipboard"]);
        command
    }
}
