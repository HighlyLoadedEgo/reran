use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "reran", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Hook adapter: invoked by harness hooks (reads JSON on stdin)
    Hook {
        #[arg(long)]
        event: String, // "pre" | "post"
    },
    /// Wire hooks into a harness config
    Init { harness: String },
    /// Show honest token-savings stats
    Gain,
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Hook { .. } | Cmd::Init { .. } | Cmd::Gain => {}
    }
}
