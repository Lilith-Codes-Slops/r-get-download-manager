pub mod sleep_inhibitor_manager;

#[cfg(target_os = "linux")]
pub mod inhibit_dbus_linux;

#[cfg(target_os = "windows")]
pub mod execution_state_windows;

pub use sleep_inhibitor_manager::SleepInhibitorManager;
