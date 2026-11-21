use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, bail};
use farrier_service::{
    Files, Job, Launchd, ProcessType, Restart, Schedule, Service, Status, Systemd,
};
use shed_client::{Conn, Error};
use shed_server::Paths;
use shed_server::proto::{BUILD, LONG_VERSION};
use shed_server::runner::ExitCode;

const LABEL: &str = "dev.shed.serve";
const UPDATE: &str = "dev.shed.update";

fn services() -> anyhow::Result<[Service; 2]> {
    let env = farrier_service::Env::host().context("this host's job manager")?;
    let exe = std::env::current_exe().context("this binary's path")?;
    let h = env.home.display();
    let path =
        format!("{h}/.cargo/bin:{h}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
    let keep = Schedule::KeepAlive(Restart::Always);
    let serve = Job::new(LABEL, [exe.as_os_str(), "serve".as_ref()], keep)?
        .path(&path)
        .launchd(Launchd {
            process_type: ProcessType::Interactive,
            abandon_process_group: false,
        })
        .systemd(Systemd {
            oom_policy_continue: true,
        })
        .supersedes(["shed"]);
    let every = Schedule::Every(Duration::from_mins(5));
    let update = Job::new(UPDATE, [exe.as_os_str(), "self-update".as_ref()], every)?
        .path(&path)
        .launchd(Launchd {
            process_type: ProcessType::Background,
            abandon_process_group: false,
        })
        .supersedes(["shed-update"]);
    Ok([Service::new(serve, &env), Service::new(update, &env)])
}

pub fn install(force: bool) -> anyhow::Result<()> {
    let services = services()?;
    for service in &services {
        if !force && service.status()?.files == Files::Differs {
            let label = service.label();
            bail!("{label} has other contents; `shed install --force` replaces it");
        }
    }
    for service in &services {
        let done = service.install()?;
        crate::out::line(format_args!("{}: {done}", service.label()));
    }
    Ok(())
}

pub fn start_job() -> anyhow::Result<bool> {
    let Ok([serve, _]) = services() else {
        return Ok(false);
    };
    if serve.files().iter().any(|(file, _)| !file.exists()) {
        return Ok(false);
    }
    serve.start()?;
    Ok(true)
}

pub fn uninstall() -> anyhow::Result<()> {
    for service in services()? {
        let done = service.uninstall()?;
        crate::out::line(format_args!("{}: {done}", service.label()));
    }
    Ok(())
}

fn on_disk(
    service: &Service,
    status: &Result<Status, farrier_service::Error>,
) -> Result<String, String> {
    let files: Vec<String> = (service.files().iter())
        .map(|(file, _)| file.display().to_string())
        .collect();
    let files = files.join(", ");
    match status.as_ref().map(|s| s.files) {
        Ok(Files::Same) => Ok(files),
        Ok(Files::Differs) => Err(format!("{files} differs; fix: shed install --force")),
        Ok(Files::Missing) => Err(format!("{files}: not written; fix: shed install")),
        Err(e) => Err(format!("{files}: {e}")),
    }
}

fn loaded(
    status: &Result<Status, farrier_service::Error>,
    outside: Option<String>,
    fix: &str,
) -> Result<String, String> {
    match (status, outside) {
        (Ok(s), _) if s.loaded && s.refused.is_none() => Ok(LABEL.into()),
        (
            Ok(Status {
                refused: Some(why), ..
            }),
            _,
        ) => Err(format!("{why}; fix: shed install")),
        (_, Some(server)) => Err(format!(
            "a server outside {LABEL} answers on {server}; fix: stop that server, then {fix}"
        )),
        _ => Err(format!("not loaded; fix: {fix}")),
    }
}

pub fn doctor(paths: &Paths) -> anyhow::Result<ExitCode> {
    let [job, update] = services()?;
    let (status, commands) = (job.status(), job.commands());
    let socket = paths.socket.display();
    let fix = format!("fix: {}", commands.load);
    let server = Conn::connect(&paths.socket);
    let outside = server.is_ok().then(|| socket.to_string());
    let checks = [
        ("job", on_disk(&job, &status)),
        ("loaded", loaded(&status, outside, &commands.load)),
        (
            "socket",
            match &server {
                Err(e) if !matches!(e, Error::Version { .. }) => {
                    Err(format!("{socket}: {e}; {fix}"))
                }
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
                Err(e @ Error::Version { .. }) => Err(format!("{e}; fix: shed install")),
                Err(_) => Err("no server to ask".into()),
            },
        ),
        (
            "update",
            on_disk(&update, &update.status()).and_then(|_| match crate::update::last(paths) {
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
            match commands.log.strip_prefix("tail -f ") {
                Some(log) if !Path::new(log).exists() => {
                    Err(format!("{log} not written yet; {fix}"))
                }
                Some(log) => Ok(log.to_owned()),
                None => Ok(commands.log.clone()),
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
    let bad = shed_server::providers::Catalog::load(&dir).1;
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
