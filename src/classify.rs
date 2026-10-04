#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Class {
    Memoizable,
    Bypass,
}

/// env assignments like FOO=bar before the command
fn strip_env<'a>(argv: &'a [String]) -> &'a [String] {
    let n = argv
        .iter()
        .take_while(|a| {
            a.contains('=')
                && !a.starts_with('-')
                && a.split('=').next().is_some_and(|k| {
                    !k.is_empty()
                        && k.chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                })
        })
        .count();
    &argv[n..]
}

/// Read-only subcommands, keyed by first token. Unknown first tokens fall through.
const READ_SUBCOMMANDS: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "status", "log", "diff", "show", "branch", "tag", "remote", "rev-parse", "describe",
            "blame", "shortlog", "ls-files", "config", "--list", "-l",
        ],
    ),
    ("npm", &["ls", "list", "outdated", "view"]),
    ("cargo", &["check", "tree", "search", "metadata"]),
];

/// find flags with side effects (rtk#3410 class: `find -delete` runs nothing under a cached answer).
const FIND_SIDE_EFFECT_FLAGS: &[&str] = &["-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint"];

/// Read-only bare commands. Unknown ⇒ Bypass (bkt#58: unknown commands may write files).
const READ_BARE: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "rg", "fd", "find", "pwd", "which", "whoami", "wc",
    "file", "tree", "du", "df", "date", "echo", "env", "printenv", "uname", "id",
];

/// Arguments we can vouch are read-flags, not operands. A rev or pattern is NOT
/// vouchable: `git tag v1` writes, `git tag -l v1` reads — M1 only trusts flags.
const SAFE_FLAGS: &[&str] = &[
    "-l", "--list", "-v", "-a", "-r", "-s", "--short", "--oneline", "--graph", "--decorate",
    "--abbrev-commit", "-n", "-q", "--quiet", "--stat", "--name-only", "--numstat",
    "--shortstat",
];

fn is_count(arg: &str) -> bool {
    let num = arg.trim_start_matches('-');
    !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
}

fn all_read_flags(args: &[String]) -> bool {
    args.iter()
        .all(|a| SAFE_FLAGS.contains(&a.as_str()) || is_count(a))
}

pub fn classify(argv: &[String]) -> Class {
    let argv = strip_env(argv);
    if argv.is_empty() {
        return Class::Bypass;
    }
    let cmd = argv[0].as_str();
    if argv.iter().any(|a| a == "&&" || a == "||" || a == ";" || a == "|") {
        return Class::Bypass; // M1: compound commands not analyzed
    }
    if argv
        .iter()
        .any(|a| a.starts_with('>') || a.starts_with('<') || a.contains(">&"))
    {
        return Class::Bypass; // shell redirects write files
    }
    // Command substitution is not computable pre-run (bkt#20) — "unchanged" would be a lie.
    if argv.iter().any(|a| a.contains('$') || a.contains('`')) {
        return Class::Bypass;
    }
    if cmd == "find" && argv.iter().any(|a| FIND_SIDE_EFFECT_FLAGS.contains(&a.as_str())) {
        return Class::Bypass;
    }
    if READ_BARE.contains(&cmd) {
        return Class::Memoizable;
    }
    if let Some((_, subs)) = READ_SUBCOMMANDS.iter().find(|(name, _)| cmd == *name) {
        match argv.get(1) {
            // subcommand not in the read list (commit/stash/push/…) or flag-first forms
            // (`git -C …`) — unanalyzed in M1
            None => return Class::Bypass,
            Some(sub) if !subs.contains(&sub.as_str()) => return Class::Bypass,
            Some(_) => {}
        }
        // Subcommand is a known reader. Bare form is a pure lister; with args, only
        // vouchable read-flags pass — `git tag v1` writes, `git tag -l` reads.
        if argv.len() == 2 || all_read_flags(&argv[2..]) {
            return Class::Memoizable;
        }
    }
    // npm run / cargo test etc. execute project code — never read-only, stay Bypass.
    Class::Bypass
}
