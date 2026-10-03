//! Secret names and protected path components shared by the gate and readers.

use std::path::Path;

/// A project's own environment file: `.env`, `.env.local`, `config/.env.production`,
/// `local.env`. Not an example of one, and not a key or a credential folder.
/// It holds the project's settings for this machine, which the build hat
/// may be asked to write; it is never read.
pub(super) fn is_dotenv(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let example = [".example", ".sample", ".template", ".dist", ".defaults"]
        .iter()
        .any(|e| name.ends_with(e));
    (name == ".env" || name.ends_with(".env") || (name.starts_with(".env") && !example))
        && is_secret(path)
        && !path.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|c| c.eq_ignore_ascii_case(".ryter"))
        })
}

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
