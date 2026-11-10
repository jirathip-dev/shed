use std::path::PathBuf;

use anyhow::{Context as _, bail};
use shed_server::{Client, Error, Paths};

const LABEL: &str = "dev.shed.serve";

struct Job {
    path: PathBuf,
    text: String,
    start: String,
    stop: String,
    log: PathBuf,
    loaded: &'static str,
}

fn job() -> anyhow::Result<Job> {
    let var = |k| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var("HOME").context("HOME is not set")?;
    let exe = std::env::current_exe().context("this binary's path")?;
    let (h, e) = (home.display(), exe.display());
    let path =
        format!("{h}/.cargo/bin:{h}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
    if cfg!(target_os = "macos") {
        let file = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
        let log = home.join("Library/Logs/shed.log");
        let l = log.display();
        let text = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key><array><string>{e}</string><string>serve</string></array>
    <key>EnvironmentVariables</key><dict><key>PATH</key><string>{path}</string></dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>ProcessType</key><string>Interactive</string>
    <key>StandardOutPath</key><string>{l}</string>
    <key>StandardErrorPath</key><string>{l}</string>
</dict>
</plist>
"#
        );
        return Ok(Job {
            start: format!("launchctl bootstrap gui/$(id -u) {}", file.display()),
            stop: format!("launchctl bootout gui/$(id -u)/{LABEL}"),
            path: file,
            text,
            log,
            loaded: "launchctl list dev.shed.serve",
        });
    }
    let config = var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    let text = format!(
        "[Unit]\nDescription=shed: keeps coding-agent terminals alive\n\n[Service]\nExecStart={e} serve\n\
         Environment=PATH={path}\nRestart=always\n\n[Install]\nWantedBy=default.target\n"
    );
    Ok(Job {
        path: config.join("systemd/user/shed.service"),
        text,
        start: "systemctl --user daemon-reload && systemctl --user enable --now shed.service"
            .into(),
        stop: "systemctl --user disable --now shed.service".into(),
        log: "journalctl --user -u shed.service".into(),
        loaded: "systemctl --user is-active --quiet shed.service",
    })
}

pub fn install(force: bool) -> anyhow::Result<()> {
    let job = job()?;
    let file = job.path.display();
    match std::fs::read_to_string(&job.path) {
        Ok(old) if old == job.text => println!("{file} is up to date"),
        Ok(_) if !force => bail!("{file} has other contents; `shed install --force` replaces it"),
        old => {
            let dir = job.path.parent().context("the job's directory")?;
            std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
            std::fs::write(&job.path, &job.text).with_context(|| file.to_string())?;
            println!("wrote {file}");
            if old.is_ok() {
                println!("stop the old job first:\n  {}", job.stop);
            }
        }
    }
    println!("start it with:\n  {}", job.start);
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let job = job()?;
    println!(
        "stop it and remove it with:\n  {}\n  rm {}",
        job.stop,
        job.path.display()
    );
    Ok(())
}

pub fn doctor(paths: &Paths) -> anyhow::Result<bool> {
    let job = job()?;
    let (file, log) = (job.path.display(), job.log.display());
    let socket = paths.socket.display();
    let fix = format!("fix: {}", job.start);
    let loaded = shed_server::runner::output("sh", ["-c", job.loaded], &[])
        .is_ok_and(|o| o.status.success());
    let server = Client::connect(&paths.socket);
    let checks = [
        (
            "job",
            match std::fs::read_to_string(&job.path) {
                Ok(text) if text == job.text => Ok(file.to_string()),
                Ok(_) => Err(format!("{file} differs; fix: shed install --force")),
                Err(e) => Err(format!("{file}: {e}; fix: shed install")),
            },
        ),
        (
            "loaded",
            if loaded {
                Ok(LABEL.into())
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
                Ok(_) => Ok(format!("shed {}", env!("CARGO_PKG_VERSION"))),
                Err(Error::Rpc(e)) => Err(format!("{e}; fix: {} && {}", job.stop, job.start)),
                Err(_) => Err("no server to ask".into()),
            },
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
        println!("{mark:<5} {what:<8} {v}");
    }
    Ok(checks.iter().all(|(_, r)| r.is_ok()))
}
