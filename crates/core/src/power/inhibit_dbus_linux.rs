#![cfg(target_os = "linux")]

pub struct InhibitDbusLinux;

impl InhibitDbusLinux {
    pub async fn acquire() -> Result<(), String> {
        // D-Bus login1 Inhibit call would be issued here.
        Ok(())
    }

    pub async fn release() -> Result<(), String> {
        Ok(())
    }
}
