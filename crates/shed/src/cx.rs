use std::collections::BTreeMap;
use std::io::{self, BufRead as _, Read as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::Serialize;
use serde_json::json;
use shed_client::{Calls as _, Conn, Shed};
use shed_server::Paths;
use shed_server::machines::{self, Machine};
use shed_server::proto::{Link, LinkState};

use crate::{out, service};

pub enum On {
    No,
    Ssh,
    Link,
}

pub struct Cx<'a> {
    pub paths: &'a Paths,
    pub dir: Option<&'a Path>,
    pub json: bool,
    pub server: Server<'a>,
}

impl Cx<'_> {
    pub fn print<T: Serialize>(
        &self,
        value: &T,
        text: impl FnOnce() -> String,
    ) -> anyhow::Result<()> {
        out::print(self.json, value, text)
    }

    pub fn to_acp(
        &self,
        name: &str,
        input: impl FnOnce(&Conn) -> Result<(), shed_client::Error>,
    ) -> anyhow::Result<()> {
        let client = self.server.connect(None)?;
        if client.get(name)?.acp.is_none() {
            bail!("{name} is not an ACP terminal (shed run --acp)");
        }
        input(&client)?;
        self.print(&json!({}), String::new)
    }
}

pub struct Server<'a> {
    pub paths: &'a Paths,
    dir: Option<&'a Path>,
    pub on: Option<Machine>,
    pub linked: bool,
}

impl Server<'_> {
    pub fn new<'a>(
        paths: &'a Paths,
        dir: Option<&'a Path>,
        on: Option<String>,
        reach: &On,
    ) -> anyhow::Result<Server<'a>> {
        if matches!(reach, On::No) && on.is_some() {
            bail!(
                "--on takes the terminal commands (run, ls, read, …), not hello, serve, connect, machine, agents, install or doctor"
            );
        }
        let on = on
            .map(|name| machines::find(&paths.config, &name))
            .transpose()?;
        let up = |m: &Machine| links(paths).get(&m.name).map(|l| l.state) == Some(LinkState::Up);
        let linked = matches!(reach, On::Link) && on.as_ref().is_some_and(up);
        Ok(Server {
            paths,
            dir,
            on,
            linked,
        })
    }

    pub fn name(&self, name: String) -> String {
        match &self.on {
            Some(m) if self.linked => format!("{}/{name}", m.name),
            _ => name,
        }
    }

    pub fn machine(&self) -> Option<String> {
        let on = self.on.as_ref().filter(|_| self.linked);
        on.map(|m| m.name.clone())
    }

    pub fn resolve(&self, path: PathBuf) -> anyhow::Result<PathBuf> {
        match &self.on {
            None => Ok(std::env::current_dir()
                .context("current directory")?
                .join(path)),
            Some(_) if path.is_absolute() => Ok(path),
            Some(m) => bail!(
                "with --on, pass {} as an absolute path on {}",
                path.display(),
                m.name
            ),
        }
    }

    pub fn connect(&self, autostart: Option<bool>) -> anyhow::Result<Conn> {
        self.connect_within(autostart, None)
    }

    pub fn connect_within(
        &self,
        autostart: Option<bool>,
        deadline: Option<Duration>,
    ) -> anyhow::Result<Conn> {
        match &self.on {
            Some(machine) if !self.linked => {
                let control = self.paths.socket.parent().unwrap_or(Path::new("."));
                Ok(machines::connect(
                    machine,
                    control,
                    autostart == Some(true),
                )?)
            }
            _ => connect(self.paths, self.dir, autostart, deadline),
        }
    }

    pub fn again<T>(
        &self,
        mut call: impl FnMut(&Conn) -> Result<T, shed_client::Error>,
    ) -> anyhow::Result<T> {
        let mut client = self.connect(None)?;
        for _ in 0..100 {
            match call(&client) {
                Err(shed_client::Error::Bad(_) | shed_client::Error::Io { .. }) => {
                    shed_server::clock::sleep(Duration::from_millis(100));
                    client = self.connect(None).unwrap_or(client);
                }
                done => return Ok(done?),
            }
        }
        Ok(call(&client)?)
    }
}

pub fn links(paths: &Paths) -> BTreeMap<String, Link> {
    Shed::at(&paths.socket).links().unwrap_or_default()
}

pub fn stdin_bytes() -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes).context("stdin")?;
    Ok(bytes)
}

pub fn connect(
    paths: &Paths,
    dir: Option<&Path>,
    autostart: Option<bool>,
    deadline: Option<Duration>,
) -> anyhow::Result<Conn> {
    let dial = UnixStream::connect(&paths.socket);
    match dial.and_then(|s| s.set_read_timeout(deadline).map(|()| s)) {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            if autostart == Some(true) {
                let started = start_server(paths, dir);
                for _ in 0..50 {
                    if let Ok(client) = Conn::connect(&paths.socket) {
                        return Ok(client);
                    }
                    shed_server::clock::sleep(Duration::from_millis(100));
                }
                started?;
                return Ok(Conn::connect(&paths.socket)?);
            }
            let hint = if autostart.is_some() {
                " (or pass --autostart)"
            } else {
                ""
            };
            bail!(
                "no shed server on {}; start one with `shed serve --detach`{hint}",
                paths.socket.display()
            )
        }
        stream => Ok(Conn::over(stream?)?),
    }
}

fn start_server(paths: &Paths, dir: Option<&Path>) -> anyhow::Result<()> {
    if dir.is_some() || !service::start_job()? {
        return serve_detached(paths, dir, None);
    }
    Ok(())
}

pub fn serve_args(dir: Option<&Path>, sample: Option<f64>) -> Vec<std::ffi::OsString> {
    let mut args = vec!["serve".into()];
    if let Some(dir) = dir {
        args.extend(["--dir".into(), dir.into()]);
    }
    if let Some(sample) = sample {
        args.extend(["--sample".into(), sample.to_string().into()]);
    }
    args
}

pub fn serve_detached(
    paths: &Paths,
    dir: Option<&Path>,
    sample: Option<f64>,
) -> anyhow::Result<()> {
    if UnixStream::connect(&paths.socket).is_ok() {
        bail!("a shed server already runs on {}", paths.socket.display());
    }
    let args = serve_args(dir, sample);
    let mut child = shed_server::runner::detached(&std::env::current_exe()?, args)
        .context("starting shed serve")?;
    let stdout = child.stdout.take().context("shed serve stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        io::BufReader::new(stdout).read_line(&mut line).ok();
        tx.send(line).ok();
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(line) => match line.trim_end().strip_prefix("shed ready ") {
            Some(ready) => out::err(format_args!(
                "shed: server {} started",
                ready.replacen(") ", ") on ", 1)
            )),
            None => bail!("shed serve exited ({}) before it was ready", child.wait()?),
        },
        Err(_) => bail!("shed serve was not ready within 10 s (still running, its stdout silent)"),
    }
    Ok(())
}
