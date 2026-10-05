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
    // pure-stdout text utilities (v1.2): no file-writing flag exists in any of
    // these. sort is DELIBERATELY absent (-o writes); mktemp/patch/tee too.
    "jq", "cut", "uniq", "diff", "stat", "realpath", "dirname", "basename", "column",
    "xxd", "sha256sum", "md5", "cksum", "nl", "strings", "base64", "seq", "paste",
    "join", "comm", "cmp", "iconv", "tac", "rev", "fmt", "fold", "expand",
];

/// sed writes only through in-place flags; every glued form starts with "-i"
/// (-i, -i.bak, -in) or is --in-place. Known residual gap: the `w` script-
/// command (zero occurrences in mined agent workloads).
fn sed_is_read_only(argv: &[String]) -> bool {
    !argv
        .iter()
        .any(|a| a.starts_with("-i") || a.starts_with("--in-place"))
}

/// tsc is cached ONLY in the --noEmit form: plain tsc emits .js, --watch
/// never exits, --incremental/--build write tsbuildinfo.
fn tsc_is_read_only(tsc_args: &[String]) -> bool {
    tsc_args.iter().any(|a| a.as_str() == "--noEmit")
        && !tsc_args.iter().any(|a| {
            a.as_str() == "--watch"
                || a.as_str() == "-w"
                || a.as_str() == "--incremental"
                || a.as_str() == "--build"
        })
}

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

/// Operators that split a command into segments we analyze independently.
const SEGMENT_OPS: &[&str] = &["|", "||", "&&", ";"];

/// fd-only redirects that touch no files. Anything else containing a
/// redirect operator writes (or reads) a path we cannot vouch for.
fn is_safe_redirect_token(token: &str) -> bool {
    matches!(token, "2>&1" | "1>&2" | "2>/dev/null" | "2>>/dev/null")
}

fn has_unsafe_redirect(argv: &[String]) -> bool {
    argv.iter()
        .any(|a| (a.contains('>') || a.contains('<')) && !is_safe_redirect_token(a))
}

/// Classify one pipe-free segment: every token must be accountably read-only.
fn classify_segment(argv: &[String]) -> Class {
    let argv = strip_env(argv);
    if argv.is_empty() {
        return Class::Bypass;
    }
    let cmd = argv[0].as_str();
    if has_unsafe_redirect(argv) {
        return Class::Bypass;
    }
    // Command substitution is not computable pre-run (bkt#20).
    if argv.iter().any(|a| a.contains('$') || a.contains('`')) {
        return Class::Bypass;
    }
    // Operators glued inside a token ("2>&1|head") are invisible to
    // segmentation — refuse rather than misread what will execute. EXCEPT
    // backslash-escaped ones: grep BRE patterns like "a\|b" are literals,
    // not shell operators. Shell operators cannot be both escaped AND active.
    if argv.iter().any(|a| {
        !is_safe_redirect_token(a) && a.len() > 1 && {
            let bare = a.replace("\\|", "").replace("\\;", "").replace("\\&", "");
            bare.contains('|') || bare.contains(';') || bare.contains('&')
        }
    }) {
        return Class::Bypass;
    }
    if cmd == "find" && argv.iter().any(|a| FIND_SIDE_EFFECT_FLAGS.contains(&a.as_str())) {
        return Class::Bypass;
    }
    if cmd == "sed" {
        return if sed_is_read_only(argv) { Class::Memoizable } else { Class::Bypass };
    }
    if cmd == "tsc" {
        return if tsc_is_read_only(&argv[1..]) { Class::Memoizable } else { Class::Bypass };
    }
    if cmd == "npx" && argv.get(1).map(|s| s.as_str()) == Some("tsc") {
        return if tsc_is_read_only(&argv[2..]) { Class::Memoizable } else { Class::Bypass };
    }
    if READ_BARE.contains(&cmd) {
        return Class::Memoizable;
    }
    if let Some((_, subs)) = READ_SUBCOMMANDS.iter().find(|(name, _)| cmd == *name) {
        match argv.get(1) {
            None => return Class::Bypass,
            Some(sub) if !subs.contains(&sub.as_str()) => return Class::Bypass,
            Some(_) => {}
        }
        if argv.len() == 2 || all_read_flags(&argv[2..]) {
            return Class::Memoizable;
        }
    }
    Class::Bypass
}

pub fn classify(argv: &[String]) -> Class {
    let argv = strip_env(argv);
    if argv.is_empty() {
        return Class::Bypass;
    }
    // Split on top-level segment operators; a pipeline is memoizable only
    // when every segment is (the cache key is the whole composed argv).
    let mut segments: Vec<Vec<String>> = vec![Vec::new()];
    for token in argv {
        if SEGMENT_OPS.contains(&token.as_str()) {
            segments.push(Vec::new());
        } else {
            segments.last_mut().unwrap().push(token.clone());
        }
    }
    segments
        .iter()
        .all(|seg| classify_segment(seg) == Class::Memoizable)
        .then_some(Class::Memoizable)
        .unwrap_or(Class::Bypass)
}
