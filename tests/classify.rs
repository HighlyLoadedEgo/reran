use reran::classify::{classify, Class};

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

#[test]
fn read_prefixes_are_memoizable() {
    for cmd in [
        vec!["git", "status"],
        vec!["git", "log", "-5"],
        vec!["git", "diff", "--stat"],
        vec!["ls", "-la"],
        vec!["cat", "README.md"],
        vec!["grep", "-r", "todo", "."],
        vec!["rg", "pattern"],
        vec!["find", ".", "-name", "x"],
        vec!["pwd"],
        vec!["which", "node"],
    ] {
        assert_eq!(classify(&v(&cmd)), Class::Memoizable, "{cmd:?}");
    }
}

#[test]
fn writes_and_unknown_are_bypass() {
    for cmd in [
        vec!["git", "commit", "-m", "x"],
        vec!["git", "add", "."],
        vec!["git", "push"],
        vec!["npm", "install"],
        vec!["rm", "-rf", "x"],
        vec!["mkdir", "x"],
        vec!["touch", "x"],
        vec!["curl", "https://example.com"],
        vec!["node", "build.js"],
        vec!["git", "status", "&&", "npm", "test"],
    ] {
        assert_eq!(classify(&v(&cmd)), Class::Bypass, "{cmd:?}");
    }
}

#[test]
fn rev_args_are_not_vouchable_in_m1() {
    // `git tag v1` writes; `git diff HEAD~1` reads but M1 can't tell operand kinds apart
    assert_eq!(classify(&v(&["git", "diff", "HEAD~1"])), Class::Bypass);
    assert_eq!(classify(&v(&["git", "tag", "-l", "v1.0"])), Class::Bypass);
    assert_eq!(classify(&v(&["git", "log", "-5"])), Class::Memoizable, "-N count is safe");
    assert_eq!(classify(&v(&["git", "branch"])), Class::Memoizable, "bare lister");
}

#[test]
fn env_assignment_prefix_is_skipped() {
    assert_eq!(classify(&v(&["FOO=bar", "git", "status"])), Class::Memoizable);
    assert_eq!(classify(&v(&["FOO=bar", "npm", "install"])), Class::Bypass);
}

#[test]
fn project_code_execution_is_bypass() {
    // npm run / cargo test execute project code — writes possible, never cached in M1
    assert_eq!(classify(&v(&["npm", "run", "build"])), Class::Bypass);
    assert_eq!(classify(&v(&["cargo", "test"])), Class::Bypass);
}

#[test]
fn mutating_lookalikes_are_bypass() {
    // `git stash` (no args) stashes = write; only `git stash list` would be read (uncached in M1)
    assert_eq!(classify(&v(&["git", "stash"])), Class::Bypass);
    // find with side-effect flags (rtk#3410 class)
    assert_eq!(classify(&v(&["find", ".", "-name", "x", "-delete"])), Class::Bypass);
    assert_eq!(classify(&v(&["find", ".", "-exec", "rm", "{}", ";"])), Class::Bypass);
    assert_eq!(classify(&v(&["find", ".", "-ok", "rm", "{}", ";"])), Class::Bypass);
    // shell redirects that write files
    assert_eq!(classify(&v(&["echo", "x", ">", "f"])), Class::Bypass);
    assert_eq!(classify(&v(&["grep", "-r", "x", ".", ">", "out.txt"])), Class::Bypass);
}

// ── Read-only pipelines: agents compose almost every read into
// `cmd | head`, `a && echo --- && b`. A pipeline is memoizable when EVERY
// segment is memoizable — the whole argv string is the cache key, so the
// composed answer is exact. Any write/unknown segment poisons the pipeline.

#[test]
fn pipelines_of_reads_are_memoizable() {
    assert_eq!(
        classify(&v(&["ls", "/x", "&&", "echo", "---", "&&", "find", "/x", "-type", "d", "|", "head", "-40"])),
        Class::Memoizable
    );
    assert_eq!(classify(&v(&["grep", "-rn", "fn", "src", "|", "head", "-20"])), Class::Memoizable);
    assert_eq!(classify(&v(&["ls", "/a", ";", "ls", "/b"])), Class::Memoizable);
}

#[test]
fn one_write_segment_poisons_the_pipeline() {
    assert_eq!(classify(&v(&["grep", "x", "f", "|", "tee", "/tmp/out"])), Class::Bypass);
    assert_eq!(classify(&v(&["ls", "/a", ";", "git", "commit", "-m", "x"])), Class::Bypass);
}

#[test]
fn fd_redirects_are_not_writes() {
    // stderr→stdout and stderr→/dev/null touch no files — agents append
    // these to nearly every command.
    assert_eq!(classify(&v(&["grep", "-rn", "fn", "src", "2>&1"])), Class::Memoizable);
    assert_eq!(classify(&v(&["ls", "/x", "2>/dev/null"])), Class::Memoizable);
    assert_eq!(classify(&v(&["ls", "/x", "2>/dev/null", "|", "head", "-5"])), Class::Memoizable);
}

#[test]
fn file_redirects_still_bypass() {
    assert_eq!(classify(&v(&["cat", "f", ">", "/tmp/out"])), Class::Bypass);
    assert_eq!(classify(&v(&["cat", "f", ">>", "/tmp/out"])), Class::Bypass);
    assert_eq!(classify(&v(&["grep", "x", "f", "2>/tmp/err.log"])), Class::Bypass);
}

#[test]
fn in_token_operators_stay_bypass() {
    // shlex keeps "2>&1|head" as one token when the pipe has no spaces —
    // segmentation cannot see it, so conservatively bypass.
    assert_eq!(classify(&v(&["grep", "x", "f", "2>&1|head", "-3"])), Class::Bypass);
}

#[test]
fn escaped_bre_alternation_is_not_an_operator() {
    // grep BRE "a\|b" is a literal alternation, not a shell pipe — agents use
    // it constantly. v1.1 regressed this into bypass; regression pinned here.
    assert_eq!(
        classify(&v(&["grep", "-n", "-i", "classif\\|read-only", "docs/specs/x.md"])),
        Class::Memoizable
    );
    assert_eq!(classify(&v(&["grep", "a\\;b", "f"])), Class::Memoizable);
    assert_eq!(classify(&v(&["grep", "a\\&b", "f"])), Class::Memoizable);
}

// ── sed: print-to-stdout forms are reads; EVERY in-place form (-i, -i '',
// -i.bak, -in, --in-place) writes the input file. "-i" prefix catches the
// glued variants. The sed `w` script-command is a known residual gap (0
// occurrences in mined agent workloads).

#[test]
fn sed_print_forms_are_memoizable() {
    assert_eq!(classify(&v(&["sed", "-n", "1,102p", "README.md"])), Class::Memoizable);
    assert_eq!(classify(&v(&["sed", "s/a/b/", "file"])), Class::Memoizable);
    assert_eq!(classify(&v(&["sed", "-n", "40,60p", "f", "|", "head", "-10"])), Class::Memoizable);
}

#[test]
fn sed_script_with_in_token_semicolon_is_bypass() {
    // shlex drops the quotes: '85,100p;305,345p' arrives as one token we cannot
    // distinguish from a top-level ';'. Conservative false-bypass, accepted.
    assert_eq!(classify(&v(&["sed", "-n", "85,100p;305,345p", "tests/x.py"])), Class::Bypass);
}

#[test]
fn sed_in_place_is_bypass() {
    assert_eq!(classify(&v(&["sed", "-i", "''", "s/a/b/", "f"])), Class::Bypass);
    assert_eq!(classify(&v(&["sed", "-i.bak", "s/a/b/", "f"])), Class::Bypass);
    assert_eq!(classify(&v(&["sed", "-in", "s/a/b/", "f"])), Class::Bypass);
    assert_eq!(classify(&v(&["sed", "--in-place", "s/a/b/", "f"])), Class::Bypass);
    assert_eq!(classify(&v(&["sed", "-n", "1p", "f", "&&", "sed", "-i", "s/a/b/", "g"])), Class::Bypass);
}

// ── pure-stdout text utilities: no file-writing flags exist in these
// (sort is DELIBERATELY absent — sort -o writes). mktemp/patch/tee stay out.

#[test]
fn text_utils_are_memoizable() {
    for cmd in [
        vec!["jq", "-r", ".users[]", "data.json"],
        vec!["cut", "-d,", "-f1", "csv"],
        vec!["uniq", "-c"],
        vec!["wc", "-l", "f"],
        vec!["diff", "a", "b"],
        vec!["stat", "f"],
        vec!["realpath", "f"],
        vec!["basename", "/a/b"],
        vec!["column", "-t", "f"],
        vec!["xxd", "f"],
        vec!["sha256sum", "f"],
        vec!["seq", "1", "10"],
        vec!["nl", "-ba", "f"],
        vec!["strings", "bin"],
        vec!["base64", "f"],
        vec!["tree", "src"],
    ] {
        assert_eq!(classify(&v(&cmd)), Class::Memoizable, "{cmd:?}");
    }
}

#[test]
fn sort_is_deliberately_bypass() {
    // sort -o FILE writes; the flag is indistinguishable from harmless args
    // without per-flag analysis, so sort stays out of the whitelist.
    assert_eq!(classify(&v(&["sort", "f"])), Class::Bypass);
}

// ── tsc: ONLY --noEmit form is cached. Plain tsc emits .js; --watch never
// exits; --incremental/--build write tsbuildinfo. npx tsc allowed (npx cache
// writes are outside the correctness contract).

#[test]
fn tsc_no_emit_is_memoizable() {
    assert_eq!(classify(&v(&["tsc", "--noEmit"])), Class::Memoizable);
    assert_eq!(classify(&v(&["tsc", "--noEmit", "-p", "tsconfig.json"])), Class::Memoizable);
    assert_eq!(classify(&v(&["npx", "tsc", "--noEmit"])), Class::Memoizable);
}

#[test]
fn tsc_writing_forms_are_bypass() {
    assert_eq!(classify(&v(&["tsc"])), Class::Bypass, "plain tsc emits .js");
    assert_eq!(classify(&v(&["npx", "tsc"])), Class::Bypass);
    assert_eq!(classify(&v(&["tsc", "--watch"])), Class::Bypass);
    assert_eq!(classify(&v(&["tsc", "-w", "--noEmit"])), Class::Bypass);
    assert_eq!(classify(&v(&["tsc", "--incremental", "--noEmit"])), Class::Bypass);
    assert_eq!(classify(&v(&["tsc", "--build"])), Class::Bypass);
}
