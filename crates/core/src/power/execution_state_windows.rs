#![cfg(target_os = "windows")]

pub struct ExecutionStateWindows;

impl ExecutionStateWindows {
    pub fn acquire() -> Result<(), String> {
        // SetThreadExecutionState would be called here.
        Ok(())
    }

    pub fn release() -> Result<(), String> {
        Ok(())
    }
}
