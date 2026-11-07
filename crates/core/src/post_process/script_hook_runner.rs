use std::process::Command;

pub struct ScriptHookRunner;

impl ScriptHookRunner {
    pub fn run(script: &str, args: &[&str]) -> Result<(), String> {
        Command::new(script)
            .args(args)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
