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
    Gain {
        /// Show recent per-command events instead of totals
        #[arg(long)]
        history: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Hook { event } => run_hook(&event),
        Cmd::Init { harness } => run_init(&harness),
        Cmd::Gain { history } => run_gain(history),
    };
    std::process::exit(code);
}

fn bar(rate_pct: f64, width: usize) -> String {
    let filled = ((rate_pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn run_gain(history: bool) -> i32 {
    match reran::store::Store::open(&reran::store::Store::default_db_path()) {
        Ok(store) => {
            if history {
                return run_gain_history(&store);
            }
            let s = store.stats().unwrap_or_default();
            let hits = s.hits;
            let misses = s.misses;
            let rate = if hits + misses > 0 {
                (hits as f64 / (hits + misses) as f64) * 100.0
            } else {
                0.0
            };
            println!("reran Token Savings");
            println!("════════════════════");
            println!();
            println!(
                "Total commands:    {} (hits {hits} · misses {misses} · bypass {} · uncached failures {})",
                hits + misses + s.bypass + s.uncached_failures + s.no_exit_code,
                s.bypass,
                s.uncached_failures
            );
            println!("  no exit code {} (payload had no confirmable exit — not cached)", s.no_exit_code);
            println!(
                "Tokens saved:      {} (counted only on replaced output, bytes/4)",
                format_number(s.tokens_saved)
            );
            println!();
            println!("Hit rate:          {rate:.1}%  {}", bar(rate, 24));
            let top = store.top_labels(5).unwrap_or_default();
            if !top.is_empty() {
                println!();
                println!("By command (top {} by savings)", top.len());
                println!("─────────────────────────────");
                for (i, (label, count, saved)) in top.iter().enumerate() {
                    println!(
                        " {:>2}. {:<38} ×{:<4} {} tok",
                        i + 1,
                        label,
                        count,
                        human_tokens(*saved)
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("reran gain: {e}");
            1
        }
    }
}

fn run_gain_history(store: &reran::store::Store) -> i32 {
    let recent = store.recent_events(10).unwrap_or_default();
    println!("reran history (last {})", recent.len());
    println!("───────────────────────");
    for (kind, saved, label) in recent {
        let saved_str = if saved > 0 {
            format!("+{} tok", human_tokens(saved))
        } else {
            "—".to_string()
        };
        println!("{kind:<7} {:<44} {saved_str}", label);
    }
    0
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
