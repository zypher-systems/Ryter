//! Secret names and protected path components shared by the gate and readers.

use std::path::Path;

/// Classify a workspace-relative path (or an absolute path outside it).
pub(super) fn is_secret(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // A public key is published, not kept: `release.pub.pem` is checked
    // into a repository so anyone can verify with it.
    let public = [".pub.pem", ".pub.key", "public.pem", "pubkey.pem"]
        .iter()
        .any(|e| name.ends_with(e));
    if name == ".env" || ((name.ends_with(".pem") || name.ends_with(".key")) && !public) {
        return true;
    }
    let s = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    // `.env.example` and its kind are what a project ships to say which
    // variables it wants: they hold no values of anybody's.
    let example = [".example", ".sample", ".template", ".dist", ".defaults"]
        .iter()
        .any(|e| name.ends_with(e));
    // A project's `.docker/` holds its Dockerfiles and server config; the
    // logins are in `config.json`. The whole of `~/.docker` is shut by the
    // home list in the policy.
    let docker_login = s == ".docker/config.json" || s.ends_with("/.docker/config.json");
    docker_login
        || s.split('/').any(|component| {
            matches!(
                component,
                ".ssh"
                    | ".gnupg"
                    | ".aws"
                    | ".azure"
                    | ".kube"
                    | ".netrc"
                    | ".npmrc"
                    | ".pypirc"
                    | ".git-credentials"
                    | ".vault-token"
            )
        })
        || s.contains("credential")
        || s.contains("/.ryter/")
        || s.ends_with(".env")
        || (name.starts_with(".env") && !example)
}
