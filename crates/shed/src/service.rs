use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use shed_server::proto::{BUILD, LONG_VERSION};
use shed_server::runner::ExitCode;
use shed_server::{Client, Error, Paths};

const LABEL: &str = "dev.shed.serve";
const UPDATE: &str = "dev.shed.update";
const PLIST: &str = include_str!("../../../dist/launchd/shed.plist");
const UPDATE_PLIST: &str = include_str!("../../../dist/launchd/dev.shed.update.plist");
const UNIT: &str = include_str!("../../../dist/systemd/shed.service");
const UPDATE_UNIT: &str = include_str!("../../../dist/systemd/shed-update.service");
const UPDATE_TIMER: &str = include_str!("../../../dist/systemd/shed-update.timer");

struct Job {
    label: &'static str,
    files: Vec<(PathBuf, String)>,
    start: String,
    kick: String,
    stop: String,
    log: PathBuf,
    loaded: String,
}

fn jobs() -> anyhow::Result<[Job; 2]> {
    let var = |k| {
        shed_server::config::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var("HOME").context("HOME is not set")?;
    let exe = std::env::current_exe().context("this binary's path")?;
    let (h, e) = (home.display(), exe.display().to_string());
    let path =
        format!("{h}/.cargo/bin:{h}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
    let fill = |text: &str, log: &Path| {
        let text = text.replace("{exe}", &e).replace("{path}", &path);
        text.replace("{log}", &log.display().to_string())
    };
    if cfg!(target_os = "macos") {
        let job = |label, text, log: &str| {
            let plist = home.join(format!("Library/LaunchAgents/{label}.plist"));
            let log = home.join("Library/Logs").join(log);
            let start = format!("launchctl bootstrap gui/$(id -u) {}", plist.display());
            Job {
                label,
                files: vec![(plist, fill(text, &log))],
                kick: format!("launchctl kickstart gui/$(id -u)/{label} 2>/dev/null || {start}"),
                start,
                stop: format!("launchctl bootout gui/$(id -u)/{label}"),
                log,
                loaded: format!("launchctl list {label}"),
            }
        };
        return Ok([
            job(LABEL, PLIST, "shed.log"),
            job(UPDATE, UPDATE_PLIST, "shed-update.log"),
        ]);
    }
    let units = var("XDG_CONFIG_HOME")
        .unwrap_or_else(|| home.join(".config"))
        .join("systemd/user");
    let job = |label, base: &str, texts: &[(&str, &str)]| {
        let unit = texts
            .last()
            .map_or(String::new(), |(ext, _)| format!("{base}.{ext}"));
        let log = PathBuf::from(format!("journalctl --user -u {base}"));
        Job {
            label,
            files: (texts.iter())
                .map(|(ext, text)| (units.join(format!("{base}.{ext}")), fill(text, &log)))
                .collect(),
            start: format!(
                "systemctl --user daemon-reload && systemctl --user enable --now {unit}"
            ),
            kick: format!(
                "systemctl --user reset-failed {unit} 2>/dev/null; systemctl --user start {unit}"
            ),
            stop: format!("systemctl --user disable --now {unit}"),
            log,
            loaded: format!("systemctl --user is-active --quiet {unit}"),
        }
    };
    let update = [("service", UPDATE_UNIT), ("timer", UPDATE_TIMER)];
    Ok([
        job(LABEL, "shed", &[("service", UNIT)]),
        job(UPDATE, "shed-update", &update),
    ])
}

pub fn install(force: bool) -> anyhow::Result<()> {
    let jobs = jobs()?;
    for (path, text) in jobs.iter().flat_map(|j| &j.files) {
        let file = path.display();
        match std::fs::read_to_string(path) {
            Ok(old) if old == *text => crate::out::line(format_args!("{file} is up to date")),
            Ok(_) if !force => {
                bail!("{file} has other contents; `shed install --force` replaces it")
            }
            _ => {
                let dir = path.parent().context("the job's directory")?;
                std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
                std::fs::write(path, text).with_context(|| file.to_string())?;
                crate::out::line(format_args!("wrote {file}"));
            }
        }
    }
    for job in &jobs {
        let restart = format!("{} 2>/dev/null; {}", job.stop, job.start);
        sh(job.label, "restarting", &restart)?;
        crate::out::line(format_args!("restarted {}:\n  {restart}", job.label));
    }
    Ok(())
}

fn sh(label: &str, what: &str, line: &str) -> anyhow::Result<()> {
    let ran = shed_server::runner::output("sh", ["-c", line], &[])?;
    if !ran.status.success() {
        bail!(
            "{what} {label} failed ({}): {}; run:\n  {line}",
            ran.status,
            String::from_utf8_lossy(&ran.stderr).trim()
        );
    }
    Ok(())
}

pub fn start_job() -> anyhow::Result<bool> {
    match jobs() {
        Ok([job, _]) if job.files.iter().all(|(p, _)| p.exists()) => {
            sh(LABEL, "starting", &job.kick).map(|()| true)
        }
        _ => Ok(false),
    }
}

pub fn uninstall() -> anyhow::Result<()> {
    crate::out::line("stop them and remove them with:");
    for job in jobs()? {
        crate::out::line(format_args!("  {}", job.stop));
        for (path, _) in &job.files {
            crate::out::line(format_args!("  rm {}", path.display()));
        }
    }
    Ok(())
}

fn written(job: &Job) -> Result<String, String> {
    let mut names = Vec::new();
    for (path, text) in &job.files {
        let file = path.display();
        match std::fs::read_to_string(path) {
            Ok(old) if old == *text => names.push(file.to_string()),
            Ok(_) => return Err(format!("{file} differs; fix: shed install --force")),
            Err(e) => return Err(format!("{file}: {e}; fix: shed install")),
        }
    }
    Ok(names.join(", "))
}

pub fn doctor(paths: &Paths) -> anyhow::Result<ExitCode> {
    let [job, update] = jobs()?;
    let log = job.log.display();
    let socket = paths.socket.display();
    let fix = format!("fix: {}", job.start);
    let loaded = shed_server::runner::output("sh", ["-c", &job.loaded], &[])
        .is_ok_and(|o| o.status.success());
    let server = Client::connect(&paths.socket);
    let outside = format!(
        "a server outside {LABEL} answers on {socket}; fix: stop that server, then {}",
        job.kick
    );
    let checks = [
        ("job", written(&job)),
        (
            "loaded",
            if loaded {
                Ok(LABEL.into())
            } else if server.is_ok() {
                Err(outside)
            } else {
                Err(format!("not loaded; {fix}"))
            },
        ),
        (
            "socket",
            match &server {
                Err(e) if !matches!(e, Error::Rpc(_)) => Err(format!("{socket}: {e}; {fix}")),
                _ => Ok(socket.to_string()),
            },
        ),
        (
            "version",
            match server {
                Ok(c) if c.server().build == BUILD => Ok(format!("shed {LONG_VERSION}")),
                Ok(c) => Err(format!(
                    "binary {BUILD}, server {}; fix: shed install",
                    c.server().label()
                )),
                Err(Error::Rpc(e)) => Err(format!("{e}; fix: shed install")),
                Err(_) => Err("no server to ask".into()),
            },
        ),
        (
            "update",
            written(&update).and_then(|_| match crate::update::last(paths) {
                Some(why) if why.starts_with("rejected") || why.starts_with("deferred") => {
                    Err(format!("{why}; see docs/self-update.md"))
                }
                why => Ok(format!(
                    "{UPDATE}: {}",
                    why.as_deref().unwrap_or("no pass yet")
                )),
            }),
        ),
        (
            "log",
            if job.log.is_relative() || job.log.exists() {
                Ok(log.to_string())
            } else {
                Err(format!("{log} not written yet; {fix}"))
            },
        ),
    ];
    for (what, result) in &checks {
        let (mark, v) = match result {
            Ok(v) => ("ok", v),
            Err(v) => ("FAIL", v),
        };
        crate::out::line(format_args!("{mark:<5} {what:<8} {v}"));
    }
    let memory = shed_server::limit::mechanism();
    crate::out::line(format_args!("ok    memory   {memory}"));
    let home = shed_server::config::var("HOME").unwrap_or_default();
    let dir = paths.providers();
    let bad = shed_server::providers::load(&dir).1;
    for e in &bad {
        crate::out::line(format_args!("FAIL  provider {e}; skipped"));
    }
    let hooks = shed_server::hooks::setup(Path::new(&home), &dir, &[], false, true);
    if let Err(e) = &hooks {
        crate::out::line(format_args!("FAIL  hooks    {e}"));
    }
    for f in hooks.as_deref().unwrap_or_default() {
        let (agent, file) = (&f.agent, f.file.display());
        let fix = f
            .after
            .as_ref()
            .map_or("", |_| " lacks them; fix: shed setup");
        let mark = if fix.is_empty() { "ok" } else { "--" };
        crate::out::line(format_args!("{mark:<5} hooks    {agent}: {file}{fix}"));
    }
    let ok = bad.is_empty() && hooks.is_ok() && checks.iter().all(|(_, r)| r.is_ok());
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
