use reran::key::{build_key, env_allowlist, uid, CallCtx};
use std::path::PathBuf;

fn ctx() -> CallCtx {
    CallCtx {
        cwd: PathBuf::from("/tmp/proj"),
        argv: vec!["git".into(), "status".into()],
        uid: 501,
        fs_epoch: 42,
    }
}

#[test]
fn identical_inputs_equal_keys() {
    let env = env_allowlist("git");
    assert_eq!(build_key(&ctx(), &env), build_key(&ctx(), &env));
}

#[test]
fn every_field_change_changes_key() {
    let env = env_allowlist("git");
    let k0 = build_key(&ctx(), &env);

    let mut c = ctx();
    c.argv = vec!["git".into(), "log".into(), "-5".into()];
    assert_ne!(build_key(&c, &env), k0, "argv");

    let mut c = ctx();
    c.argv = vec!["git".into(), "status".into(), "--short".into()];
    assert_ne!(build_key(&c, &env), k0, "argv extension");

    let mut c = ctx();
    c.cwd = PathBuf::from("/tmp/other");
    assert_ne!(build_key(&c, &env), k0, "cwd");

    let mut c = ctx();
    c.uid = 0;
    assert_ne!(build_key(&c, &env), k0, "uid (bkt#35 sudo lesson)");

    let mut c = ctx();
    c.fs_epoch = 43;
    assert_ne!(build_key(&c, &env), k0, "fs_epoch");

    let env2 = vec![("GIT_DIR".to_string(), "/somewhere".to_string())];
    assert_ne!(build_key(&ctx(), &env2), k0, "env");
}

#[test]
fn env_allowlist_is_scoped_never_path() {
    let kube = env_allowlist("kubectl");
    assert!(kube.iter().any(|(k, _)| k == "KUBECONFIG"));
    let any = env_allowlist("totally-unknown-cmd");
    assert!(any.iter().all(|(k, _)| k != "PATH"), "never PATH in key");
    assert!(any
        .iter()
        .any(|(k, _)| k == "VIRTUAL_ENV" || k == "CONDA_PREFIX"));
}

#[test]
fn uid_returns_real_uid() {
    assert_eq!(uid(), unsafe { libc::getuid() });
}
