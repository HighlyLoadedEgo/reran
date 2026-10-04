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

pub fn classify(argv: &[String]) -> Class {
    let argv = strip_env(argv);
    if argv.is_empty() {
        return Class::Bypass;
    }
    let cmd = argv[0].as_str();
    if argv.iter().any(|a| a == "&&" || a == "||" || a == ";" || a == "|") {
        return Class::Bypass; // M1: compound commands not analyzed
    }
    if argv.iter().any(|a| a.starts_with('>') || a.starts_with('<') || a.contains(">&")) {
        return Class::Bypass; // shell redirects write files
    }
    if cmd == "find" && argv.iter().any(|a| FIND_SIDE_EFFECT_FLAGS.contains(&a.as_str())) {
        return Class::Bypass;
    }
    if READ_BARE.contains(&cmd) {
        return Class::Memoizable;
    }
    if let Some((_, subs)) = READ_SUBCOMMANDS.iter().find(|(name, _)| cmd == *name) {
        if let Some(sub) = argv.get(1) {
            let bare = sub.trim_start_matches('-');
            if subs.contains(&sub.as_str()) || subs.contains(&bare) {
                return Class::Memoizable;
            }
        }
    }
    // npm run / cargo test etc. execute project code — never read-only, stay Bypass.
    Class::Bypass
}
