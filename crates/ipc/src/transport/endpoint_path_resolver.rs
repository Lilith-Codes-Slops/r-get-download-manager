use crate::IpcResult;
use std::env;
use std::path::PathBuf;

/// Resolves the platform-native IPC endpoint path.
pub struct EndpointPathResolver;

impl EndpointPathResolver {
    /// Resolve the socket or named-pipe path for the current platform.
    pub fn resolve() -> IpcResult<String> {
        #[cfg(target_os = "linux")]
        {
            Self::resolve_linux()
        }
        #[cfg(target_os = "windows")]
        {
            Ok(r"\\.\pipe\rget-daemon".to_string())
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(IpcError::EndpointResolution(
                "unsupported platform".into(),
            ))
        }
    }

    #[cfg(target_os = "linux")]
    fn resolve_linux() -> IpcResult<String> {
        if let Ok(xdg) = env::var("XDG_RUNTIME_DIR") {
            if !xdg.is_empty() {
                let path = PathBuf::from(xdg).join("rget.sock");
                return Ok(path.to_string_lossy().into_owned());
            }
        }
        // A shared /tmp socket lets other users interfere with or impersonate
        // the daemon. Use a private per-user directory when XDG is unavailable.
        use std::os::unix::fs::PermissionsExt;
        let home = env::var_os("HOME").ok_or_else(|| crate::IpcError::EndpointResolution("HOME and XDG_RUNTIME_DIR are not set".into()))?;
        let dir = PathBuf::from(home).join(".cache").join("rget");
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        Ok(dir.join("rget.sock").to_string_lossy().into_owned())
    }

    /// Return the path as a PathBuf (Linux) or raw string (Windows).
    pub fn resolve_path() -> IpcResult<PathBuf> {
        let s = Self::resolve()?;
        Ok(PathBuf::from(s))
    }
}
