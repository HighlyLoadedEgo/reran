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
        Cmd::Init { harness } => run_init(&harness),
        Cmd::Gain => run_gain(),
    };
    std::process::exit(code);
}

fn run_gain() -> i32 {
    match reran::store::Store::open(&reran::store::Store::default_db_path()) {
        Ok(store) => {
            let s = store.stats().unwrap_or_default();
            let hits = s.hits;
            let misses = s.misses;
            let bypass = s.bypass;
            let fails = s.uncached_failures;
            let rate = if hits + misses > 0 {
                (hits as f64 / (hits + misses) as f64) * 100.0
            } else {
                0.0
            };
            println!("reran gain");
            println!("  hits {hits} · misses {misses} · bypass {bypass} · uncached failures {fails}");
            println!(
                "  tokens saved: {} (counted only on replaced output, bytes/4)",
                format_number(s.tokens_saved)
            );
            println!("  hit rate: {rate:.1}%");
            0
        }
        Err(e) => {
            eprintln!("reran gain: {e}");
            1
        }
    }
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn run_init(harness: &str) -> i32 {
    match harness {
        "claude-code" => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            let settings = std::path::Path::new(&home)
                .join(".claude")
                .join("settings.json");
            let bin = std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(String::from))
                .unwrap_or_else(|| "reran".into());
            match reran::initcmd::init_claude_code(&settings, &bin) {
                Ok(()) => {
                    println!("wired reran hooks into {}", settings.display());
                    0
                }
                Err(e) => {
                    eprintln!("reran init: {e}");
                    1
                }
            }
        }
        other => {
            eprintln!("reran init: unknown harness {other:?} (supported: claude-code)");
            1
        }
    }
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
