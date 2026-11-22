//! Where `self-update` looks.

use super::var;

/// Where `self-update` reads main's head and the releases: `$SHED_UPDATE_REPO` (a git remote) and
/// `$SHED_UPDATE_RELEASES` (a URL whose `<tag>/<asset>` is a release asset), else shed's GitHub repo.
#[must_use]
pub fn update_source() -> (String, String) {
    let repo = "https://github.com/jirathip-dev/shed";
    let repo_var = var("SHED_UPDATE_REPO").unwrap_or_else(|| format!("{repo}.git"));
    let releases =
        var("SHED_UPDATE_RELEASES").unwrap_or_else(|| format!("{repo}/releases/download"));
    (repo_var, releases)
}
