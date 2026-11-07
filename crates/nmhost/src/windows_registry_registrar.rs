#![cfg(target_os = "windows")]

pub struct WindowsRegistryRegistrar;

impl WindowsRegistryRegistrar {
    pub fn register(host_path: &str) -> Result<(), String> {
        // HKCU\Software\Google\Chrome\NativeMessagingHosts\rget would be written here.
        let _ = host_path;
        Ok(())
    }
}
