#![forbid(unsafe_code)]

use clap::Parser;
use downloader_ipc::transport::DaemonAutospawner;
use std::path::PathBuf;

mod args_parser;
mod commands;
mod ipc_client;
mod ui_render;

fn locate_daemon() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("downloader-core");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    for rel in ["target/release/downloader-core", "target/debug/downloader-core"] {
        let p = PathBuf::from(rel);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("downloader-core")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = args_parser::Cli::parse();
    let spawner = DaemonAutospawner::new(locate_daemon());
    spawner.ensure_running().await?;
    match args.command {
        args_parser::Commands::Add(cmd) => commands::cmd_add_url::run(cmd).await?,
        args_parser::Commands::List => commands::cmd_list_queue::run().await?,
        args_parser::Commands::Pause(cmd) => commands::cmd_pause_resume::pause(cmd).await?,
        args_parser::Commands::Resume(cmd) => commands::cmd_pause_resume::resume(cmd).await?,
        args_parser::Commands::Refresh(cmd) => commands::cmd_refresh_url::run(cmd).await?,
        args_parser::Commands::Rm(cmd) => commands::cmd_remove_task::run(cmd).await?,
        args_parser::Commands::Limit(cmd) => commands::cmd_speed_limit::run(cmd).await?,
        args_parser::Commands::Mirror(cmd) => commands::cmd_mirror_manage::run(cmd).await?,
    }
    Ok(())
}