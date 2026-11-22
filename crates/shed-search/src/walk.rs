use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::ignore::{self, Rule};
use crate::{Error, Limit, Limits, Over};

const BINARY_PROBE: usize = 8192;

pub(crate) fn read(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let file = fs::File::open(path).map_err(io)?;
    if file.metadata().map_err(io)?.len() > max {
        let at = path.display().to_string();
        return Err(Error::Limit(Over {
            limit: Limit::Bytes,
            max,
            at,
        }));
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes).map_err(io)?;
    Ok(bytes)
}

pub(crate) type Visit<'v> = dyn FnMut(&str, &[u8]) -> Result<(), Error> + 'v;

pub(crate) struct Tree<'a> {
    pub(crate) root: &'a Path,
    pub(crate) hidden: bool,
    pub(crate) globs: &'a [String],
    pub(crate) limits: &'a Limits,
    pub(crate) expired: &'a dyn Fn() -> bool,
}

struct Walk<'a> {
    tree: &'a Tree<'a>,
    globs: Vec<Rule>,
    rules: Vec<Rule>,
    files: usize,
    bytes: u64,
}

pub(crate) fn walk(tree: &Tree, visit: &mut Visit) -> Result<usize, Error> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", tree.root.display()));
    let root = fs::canonicalize(tree.root).map_err(io)?;
    let mut w = Walk {
        tree,
        globs: Vec::new(),
        rules: Vec::new(),
        files: 0,
        bytes: 0,
    };
    for g in tree.globs {
        ignore::parse(g, "", &mut w.globs);
    }
    if root.is_file() {
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        w.file(&root, &name, visit)?;
    } else {
        let top_rel = w.above(&root);
        w.dir(&root, &top_rel, "", visit)?;
    }
    Ok(w.files)
}

impl Walk<'_> {
    fn above(&mut self, root: &Path) -> String {
        let Some(top) = root.ancestors().find(|d| d.join(".git").exists()) else {
            return String::new();
        };
        let git = top.join(".git");
        let git_dir = match fs::read_to_string(&git) {
            Ok(file) => {
                let dir = top.join(file.trim().strip_prefix("gitdir: ").unwrap_or_default());
                let common = fs::read_to_string(dir.join("commondir")).unwrap_or_default();
                dir.join(common.trim())
            }
            Err(_) => git,
        };
        if let Ok(text) = fs::read_to_string(git_dir.join("info/exclude")) {
            ignore::parse(&text, "", &mut self.rules);
        }
        let mut base = String::new();
        let mut dir = top.to_path_buf();
        for part in root.strip_prefix(top).unwrap_or(root).components() {
            if let Ok(text) = fs::read_to_string(dir.join(".gitignore")) {
                ignore::parse(&text, &base, &mut self.rules);
            }
            dir.push(part);
            base.push_str(&part.as_os_str().to_string_lossy());
            base.push('/');
        }
        base
    }

    fn check(&self, at: &str) -> Result<(), Error> {
        if (self.tree.expired)() {
            return Err(over(
                Limit::Deadline,
                self.tree.limits.deadline.as_secs(),
                at,
            ));
        }
        Ok(())
    }

    fn wanted(&self, rel: &str) -> bool {
        let includes = self.globs.iter().any(|g| !g.negated);
        ignore::last(&self.globs, rel, false).map_or(!includes, |g| !g.negated)
    }

    fn dir(
        &mut self,
        dir: &Path,
        top_rel: &str,
        rel: &str,
        visit: &mut Visit,
    ) -> Result<(), Error> {
        self.check(rel)?;
        let kept = self.rules.len();
        if let Ok(text) = fs::read_to_string(dir.join(".gitignore")) {
            ignore::parse(&text, top_rel, &mut self.rules);
        }
        let io = |e: std::io::Error| Error::Io(format!("{}: {e}", dir.display()));
        let mut entries: Vec<_> = fs::read_dir(dir).map_err(io)?.flatten().collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let hidden = name.starts_with('.') && !self.tree.hidden;
            if name == ".git" || hidden || kind.is_symlink() {
                continue;
            }
            let (in_top, in_root) = (format!("{top_rel}{name}"), format!("{rel}{name}"));
            if ignore::ignored(&self.rules, &in_top, kind.is_dir()) {
                continue;
            }
            if kind.is_dir() {
                let dropped = self
                    .globs
                    .iter()
                    .any(|g| g.negated && g.matches(&in_root, true));
                if !dropped {
                    let path = entry.path();
                    self.dir(&path, &format!("{in_top}/"), &format!("{in_root}/"), visit)?;
                }
            } else if kind.is_file() && self.wanted(&in_root) {
                self.file(&entry.path(), &in_root, visit)?;
            }
        }
        self.rules.truncate(kept);
        Ok(())
    }

    fn file(&mut self, path: &PathBuf, rel: &str, visit: &mut Visit) -> Result<(), Error> {
        self.check(rel)?;
        self.files += 1;
        let limits = self.tree.limits;
        if self.files > limits.files {
            let max = u64::try_from(limits.files).unwrap_or(u64::MAX);
            return Err(over(Limit::Files, max, rel));
        }
        let Ok(mut file) = fs::File::open(path) else {
            return Ok(());
        };
        let size = file.metadata().map_or(0, |m| m.len());
        self.bytes = self.bytes.saturating_add(size);
        if self.bytes > limits.bytes {
            return Err(over(Limit::Bytes, limits.bytes, rel));
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return Ok(());
        }
        if bytes.iter().take(BINARY_PROBE).any(|&b| b == 0) {
            return Ok(());
        }
        visit(rel, &bytes)
    }
}

fn over(limit: Limit, max: u64, at: &str) -> Error {
    Error::Limit(Over {
        limit,
        max,
        at: at.to_owned(),
    })
}
