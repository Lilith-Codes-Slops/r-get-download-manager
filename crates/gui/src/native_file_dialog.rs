//! Native file selection without a browser or a connection to the downloader.
//!
//! On Linux the desktop's GTK (zenity) or KDE (kdialog) dialog helper is
//! required. macOS and Windows use their standard system dialog facilities.
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

pub type PickResult = Result<Option<PathBuf>, String>;

/// Run the potentially blocking system dialog off the UI thread.
pub fn pick_torrent(ctx: eframe::egui::Context) -> Receiver<PickResult> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(pick());
        ctx.request_repaint();
    });
    receiver
}

#[cfg(target_os = "linux")]
fn pick() -> PickResult {
    let candidates: &[(&str, &[&str])] = &[
        (
            "zenity",
            &[
                "--file-selection",
                "--title=Add torrent",
                "--file-filter=Torrent files | *.torrent",
            ],
        ),
        (
            "kdialog",
            &[
                "--title",
                "Add torrent",
                "--getopenfilename",
                "",
                "*.torrent|Torrent files",
            ],
        ),
    ];
    for (program, args) in candidates {
        match Command::new(program).args(*args).output() {
            Ok(output) => return parse_output(output),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not open the file dialog: {error}")),
        }
    }
    Err("A native file dialog needs zenity or kdialog installed.".into())
}

#[cfg(target_os = "macos")]
fn pick() -> PickResult {
    let output = Command::new("osascript")
        .args([
            "-e",
            "POSIX path of (choose file with prompt \"Add torrent\" of type {\"torrent\"})",
        ])
        .output()
        .map_err(|error| format!("Could not open the file dialog: {error}"))?;
    if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("-128") {
        return Ok(None);
    }
    parse_output(output)
}

#[cfg(target_os = "windows")]
fn pick() -> PickResult {
    let script = r#"Add-Type -AssemblyName System.Windows.Forms; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Title = 'Add torrent'; $d.Filter = 'Torrent files (*.torrent)|*.torrent'; if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::Write($d.FileName) }; $d.Dispose()"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", script])
        .output()
        .map_err(|error| format!("Could not open the file dialog: {error}"))?;
    parse_output(output)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn pick() -> PickResult {
    Err("A native file dialog is not supported on this platform.".into())
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn parse_output(output: std::process::Output) -> PickResult {
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        if output.status.code() == Some(1) && message.trim().is_empty() {
            return Ok(None);
        }
        return Err(format!("File dialog failed: {}", message.trim()));
    }
    // Dialogs return one path, terminated by a line ending. Preserve spaces.
    let path = String::from_utf8(output.stdout)
        .map_err(|_| "The selected file path is not valid UTF-8.".to_owned())?;
    let path = path.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(path)))
    }
}

#[cfg(all(test, unix, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Output;

    fn output(code: i32, stdout: &[u8], stderr: &[u8]) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        }
    }

    #[test]
    fn picker_preserves_spaces_and_handles_cancellation() {
        assert_eq!(
            parse_output(output(0, b"/tmp/a file.torrent\n", b"")).unwrap(),
            Some(PathBuf::from("/tmp/a file.torrent"))
        );
        assert_eq!(parse_output(output(1, b"", b"")).unwrap(), None);
    }

    #[test]
    fn picker_reports_real_errors() {
        assert!(parse_output(output(1, b"", b"cannot open display")).is_err());
    }
}
