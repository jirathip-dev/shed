use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, bail};
use farrier_service::{
    Commands, Files, Job, Launchd, ProcessType, Restart, Schedule, Service, Status, Systemd,
};
use shed_client::{Conn, Error};
use shed_server::Paths;

include!(concat!(env!("OUT_DIR"), "/doctor.rs"));

pub const LABEL: &str = "dev.shed.serve";
pub const UPDATE: &str = "dev.shed.update";
const OLD_UPDATE: &str = "shed-update";

pub enum Mark {
    Ok,
    Fail,
    Note,
}

pub struct Row {
    pub mark: Mark,
    pub what: &'static str,
    pub text: String,
}

impl Row {
    pub fn of(what: &'static str, result: Result<String, String>) -> Self {
        let (mark, text) = match result {
            Ok(text) => (Mark::Ok, text),
            Err(text) => (Mark::Fail, text),
        };
        Self { mark, what, text }
    }
}

pub struct Checks<'a> {
    pub paths: &'a Paths,
    pub job: Service,
    pub update: Service,
    pub status: Result<Status, farrier_service::Error>,
    pub commands: Commands,
    pub server: Result<Conn, Error>,
    pub fix: String,
}

pub fn doctor(paths: &Paths) -> anyhow::Result<Vec<Row>> {
    let [job, update] = services(true)?;
    let (status, commands) = (job.status(), job.commands());
    let fix = format!("fix: {}", commands.load);
    let server = Conn::connect(&paths.socket);
    let cx = Checks {
        paths,
        job,
        update,
        status,
        commands,
        server,
        fix,
    };
    Ok(ROWS.iter().flat_map(|rows| rows(&cx)).collect())
}

fn services(drop_old: bool) -> anyhow::Result<[Service; 2]> {
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
        .supersedes(drop_old.then_some(OLD_UPDATE));
    Ok([Service::new(serve, &env), Service::new(update, &env)])
}

pub fn install(force: bool) -> anyhow::Result<()> {
    let services = services(true)?;
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
    let Ok([serve, _]) = services(true) else {
        return Ok(false);
    };
    if serve.files().iter().any(|(file, _)| !file.exists()) {
        return Ok(false);
    }
    serve.start()?;
    Ok(true)
}

pub fn uninstall() -> anyhow::Result<()> {
    for service in services(true)? {
        let done = service.uninstall()?;
        crate::out::line(format_args!("{}: {done}", service.label()));
    }
    Ok(())
}

pub fn old_updater(update: &Service) -> Vec<PathBuf> {
    let old = update.files().into_iter().filter_map(|(file, _)| {
        let name = file.file_name()?.to_str()?.replacen(UPDATE, OLD_UPDATE, 1);
        Some(file.with_file_name(name))
    });
    old.filter(|file| file.exists()).collect()
}

pub fn migrate_updater() -> anyhow::Result<()> {
    let [_, update] = services(true)?;
    if old_updater(&update).is_empty() {
        return Ok(());
    }
    let [_, fresh] = services(false)?;
    fresh.install()?;
    update.install()?;
    Ok(())
}

pub fn on_disk(
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
