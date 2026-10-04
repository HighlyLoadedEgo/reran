use serde::Serialize;
use sha2::Digest;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct CallCtx {
    pub cwd: PathBuf,
    pub argv: Vec<String>,
    pub uid: u32,
    pub fs_epoch: u64,
}

pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Per-command env allowlist — every var the output may depend on, and nothing else.
/// Never PATH, never the whole environment (ccache#1790: over-keying ⇒ false misses).
const ENV_ALLOWLIST: &[(&str, &[&str])] = &[
    ("git", &["GIT_DIR", "GIT_WORK_TREE", "GIT_CONFIG_GLOBAL"]),
    ("kubectl", &["KUBECONFIG"]),
    ("aws", &["AWS_PROFILE", "AWS_DEFAULT_PROFILE", "AWS_REGION"]),
    ("docker", &["DOCKER_HOST", "DOCKER_CONTEXT"]),
    ("gh", &["GH_TOKEN_PRESENT", "GH_REPO"]),
    ("gcloud", &["CLOUDSDK_CORE_PROJECT", "GOOGLE_APPLICATION_CREDENTIALS_PRESENT"]),
    ("rg", &["RIPGREP_CONFIG_PATH"]),
    ("grep", &["GREP_OPTIONS"]),
    ("*", &["VIRTUAL_ENV", "CONDA_PREFIX", "TZ", "LANG", "LC_ALL"]),
];

fn resolved(name: &str) -> String {
    // Unset MUST be distinguishable from any value (bkt#35/#482 class: under-keying
    // ⇒ stale answers when the var appears later). "<unset>" cannot collide with a value.
    match std::env::var(name) {
        Ok(v) => v,
        Err(_) => "<unset>".to_string(),
    }
}

pub fn env_allowlist(argv0: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (cmds, vars) in ENV_ALLOWLIST {
        if *cmds != argv0 && *cmds != "*" {
            continue;
        }
        for v in *vars {
            // "GH_TOKEN_PRESENT"-style placeholders: key on presence, never the secret value
            if v.ends_with("_PRESENT") {
                let real = v.trim_end_matches("_PRESENT");
                let present = std::env::var(real).is_ok_and(|val| !val.is_empty());
                out.push((
                    v.to_string(),
                    if present { "set".to_string() } else { "<unset>".to_string() },
                ));
            } else {
                out.push((v.to_string(), resolved(v)));
            }
        }
    }
    out.sort();
    out
}

pub fn build_key(ctx: &CallCtx, env: &[(String, String)]) -> [u8; 32] {
    let canonical = serde_json::json!({
        "cwd": ctx.cwd.to_string_lossy(),
        "argv": ctx.argv,
        "uid": ctx.uid,
        "env": env,
        "fs_epoch": ctx.fs_epoch,
    });
    let mut hasher = sha2::Sha256::new();
    hasher.update(canonical.to_string().as_bytes());
    hasher.finalize().into()
}
