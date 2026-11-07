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
    Pause(TaskArgs),
    Resume(TaskArgs),
    Refresh(RefreshArgs),
    Rm(RmArgs),
    Limit(LimitArgs),
    Mirror(MirrorArgs),
}

#[derive(Parser)]
pub struct AddArgs {
    pub url: String,
    #[arg(long)]
    pub mirrors: Vec<String>,
    #[arg(long, default_value = "8")]
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
