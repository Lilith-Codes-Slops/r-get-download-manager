use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "dl", about = "rget download manager CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Add(AddArgs),
    List,
    /// Watch opt-in torrent stall and recovery events.
    TorrentEvents,
    Pause(TaskArgs),
    Resume(TaskArgs),
    Refresh(RefreshArgs),
    Rm(RmArgs),
    Limit(LimitArgs),
    Mirror(MirrorArgs),
    Verify(VerifyArgs),
}

#[derive(Parser)]
pub struct AddArgs {
    pub url: String,
    #[arg(long)]
    pub mirrors: Vec<String>,
    #[arg(long, default_value = "1")]
    pub connections: u8,
    #[arg(long)]
    pub extract: bool,
}

#[derive(Parser)]
pub struct TaskArgs {
    pub task_id: String,
}

#[derive(Parser)]
pub struct RefreshArgs {
    pub task_id: String,
    pub new_url: String,
}

#[derive(Parser)]
pub struct RmArgs {
    pub task_id: String,
    #[arg(long)]
    pub delete_file: bool,
}

#[derive(Parser)]
pub struct LimitArgs {
    pub kbps: u64,
    pub task_id: Option<String>,
}

#[derive(Parser)]
pub struct MirrorArgs {
    pub task_id: String,
    pub mirror_url: String,
}

#[derive(Parser)]
pub struct VerifyArgs {
    pub task_id: String,
    /// sha256 or blake3
    pub algorithm: String,
    /// Expected 64-character hexadecimal digest
    pub expected: String,
}
