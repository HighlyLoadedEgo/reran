use reran::classify::{classify, Class};

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

#[test]
fn read_prefixes_are_memoizable() {
    for cmd in [
        vec!["git", "status"],
        vec!["git", "log", "-5"],
        vec!["git", "diff", "HEAD~1"],
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
    // shell redirects in argv
    assert_eq!(classify(&v(&["echo", "x", ">", "f"])), Class::Bypass);
    assert_eq!(classify(&v(&["grep", "-r", "x", ".", ">", "out.txt"])), Class::Bypass);
    assert_eq!(classify(&v(&["ls", "2>&1"])), Class::Bypass);
}
