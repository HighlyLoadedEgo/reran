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
    let code = match cli.cmd {
        Cmd::Hook { event } => run_hook(&event),
        Cmd::Init { .. } | Cmd::Gain => 0,
    };
    std::process::exit(code);
}

/// Hook entry: read payload from stdin, answer on stdout. Fail-open to exit 0:
/// a broken reran must never block the agent (spec §5.6).
fn run_hook(event: &str) -> i32 {
    let result = std::panic::catch_unwind(|| {
        use std::io::Read;
        let mut input = String::new();
        if std::io::stdin().read_to_string(&mut input).is_err() {
            return 0;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let out = match event {
            "pre" => reran::hooks::hook_pre(&input, &cwd, None),
            "post" => reran::hooks::hook_post(&input, &cwd, None),
            _ => String::new(),
        };
        if !out.is_empty() {
            println!("{out}");
        }
        0
    });
    result.unwrap_or(0)
}
