pub struct DesktopNotifier;

impl DesktopNotifier {
    pub fn notify(title: &str, body: &str) {
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("notify-send").arg(title).arg(body).status();
        }
        #[cfg(target_os = "windows")]
        {
            // Win32 toast would be issued here.
            let _ = (title, body);
        }
    }
}
