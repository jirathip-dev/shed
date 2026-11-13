//! The `shed` binary: argument parsing and wiring only.
#![forbid(unsafe_code)]

mod attach;
mod out;
mod service;
mod watch;

use shed_server::runner::ExitCode;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, BufRead as _, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::{Map, Value};
use shed_server::acp::{self, Input, Policy};
use shed_server::agents;
use shed_server::machines::{self, Machine};
use shed_server::proto::{
    Children, Event, EventsParams, ExecParams, ExecResult, FileParams, KeysParams, Link, LinkState,
    LsParams, MachineStats, Method, ModelParams, Parent, ReadParams, ReadResult, Reported,
    RunParams, SendParams, State, StateParams, StatsParams, Target, TerminalInfo, WaitOutcome,
    WaitParams, WaitResult,
};
use shed_server::{Client, Paths};

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, value_name = "MACHINE")]
    on: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Hello,
    Schema,
    Serve {
        #[arg(long)]
        detach: bool,
        #[arg(long, value_name = "S", default_value_t = 2.0)]
        sample: f64,
    },
    Run(Box<RunArgs>),
    Ls {
        #[arg(long = "label", value_name = "K=V", value_parser = label)]
        labels: Vec<(String, String)>,
        #[arg(long, value_name = "T")]
        parent: Option<String>,
    },
    Get {
        name: String,
    },
    Read {
        name: String,
        #[arg(long)]
        lines: Option<usize>,
        #[arg(long, conflicts_with = "history")]
        screen: bool,
        #[arg(long)]
        history: bool,
        #[arg(long, conflicts_with_all = ["history", "screen"])]
        events: bool,
        #[arg(long, requires = "events")]
        before: Option<u64>,
    },
    Send {
        name: String,
        text: String,
    },
    Keys {
        name: String,
        #[arg(required = true)]
        keys: Vec<String>,
    },
    Kill {
        name: String,
    },
    Rm {
        name: String,
    },
    Attach {
        name: String,
    },
    State {
        #[arg(required = true, num_args = 1..=2, value_name = "[T] STATE")]
        args: Vec<String>,
        #[arg(long)]
        note: Option<String>,
    },
    Wait(WaitArgs),
    Events {
        #[arg(long = "label", value_name = "K=V", value_parser = label)]
        labels: Vec<(String, String)>,
        #[arg(long, value_name = "N")]
        since: Option<u64>,
    },
    Top,
    Watch {
        #[arg(long = "label", value_name = "K=V", value_parser = label)]
        labels: Vec<(String, String)>,
    },
    Approve {
        name: String,
        option: String,
    },
    Cancel {
        name: String,
    },
    Model {
        name: String,
        model: String,
    },
    Agents {
        #[command(subcommand)]
        action: Option<AgentsCommand>,
    },
    #[command(hide = true)]
    AcpBridge {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        state: PathBuf,
        #[arg(long, value_parser = policy)]
        policy: Policy,
        #[arg(long)]
        agent: String,
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    Connect {
        #[arg(long)]
        autostart: bool,
    },
    #[command(subcommand)]
    Machine(MachineCommand),
    Exec(ExecArgs),
    File {
        #[arg(value_parser = ["read", "write"])]
        action: String,
        path: PathBuf,
    },
    Install {
        #[arg(long)]
        force: bool,
    },
    Uninstall,
    Doctor,
}

impl Command {
    fn terminal(&mut self) -> Option<&mut String> {
        match self {
            Self::Get { name }
            | Self::Read { name, .. }
            | Self::Send { name, .. }
            | Self::Keys { name, .. }
            | Self::Kill { name }
            | Self::Rm { name }
            | Self::Approve { name, .. }
            | Self::Cancel { name }
            | Self::Model { name, .. }
            | Self::Wait(WaitArgs {
                name: Some(name), ..
            }) => Some(name),
            _ => None,
        }
    }
}

#[derive(Subcommand)]
enum AgentsCommand {
    Add {
        id: String,
        #[arg(long, value_name = "SRC", default_value = agents::REGISTRY)]
        registry: String,
    },
    Rm {
        id: String,
    },
}

#[derive(Subcommand)]
enum MachineCommand {
    Add {
        name: String,
        target: String,
    },
    Ls,
    Rm {
        name: String,
    },
    Stats {
        #[arg(long, value_name = "T")]
        since: Option<u64>,
    },
}

struct Server<'a> {
    paths: &'a Paths,
    dir: Option<&'a Path>,
    on: Option<Machine>,
    linked: bool,
}

impl Server<'_> {
    fn new<'a>(
        paths: &'a Paths,
        dir: Option<&'a Path>,
        on: Option<String>,
        command: &mut Command,
    ) -> anyhow::Result<Server<'a>> {
        let local = matches!(
            command,
            Command::Hello
                | Command::Schema
                | Command::Serve { .. }
                | Command::Connect { .. }
                | Command::Machine(
                    MachineCommand::Add { .. } | MachineCommand::Ls | MachineCommand::Rm { .. }
                )
                | Command::Agents { .. }
                | Command::AcpBridge { .. }
                | Command::Install { .. }
                | Command::Uninstall
                | Command::Doctor
        );
        if local && on.is_some() {
            bail!(
                "--on takes the terminal commands (run, ls, read, …), not hello, serve, connect, machine, agents, install or doctor"
            );
        }
        let on = on
            .map(|name| machines::find(&paths.config, &name))
            .transpose()?;
        let linkable = matches!(
            command,
            Command::Run(_)
                | Command::Ls { .. }
                | Command::Get { .. }
                | Command::Read { .. }
                | Command::Send { .. }
                | Command::Keys { .. }
                | Command::Kill { .. }
                | Command::Rm { .. }
                | Command::State { .. }
                | Command::Model { .. }
                | Command::Approve { .. }
                | Command::Cancel { .. }
                | Command::Events { .. }
                | Command::Wait(_)
                | Command::Exec(_)
                | Command::File { .. }
        );
        let up = |m: &Machine| links(paths).get(&m.name).map(|l| l.state) == Some(LinkState::Up);
        let linked = linkable && on.as_ref().is_some_and(up);
        let server = Server {
            paths,
            dir,
            on,
            linked,
        };
        if let Some(name) = command.terminal() {
            *name = server.name(std::mem::take(name));
        }
        Ok(server)
    }

    fn name(&self, name: String) -> String {
        match &self.on {
            Some(m) if self.linked => format!("{}/{name}", m.name),
            _ => name,
        }
    }

    fn machine(&self) -> Option<String> {
        let on = self.on.as_ref().filter(|_| self.linked);
        on.map(|m| m.name.clone())
    }

    fn resolve(&self, path: PathBuf) -> anyhow::Result<PathBuf> {
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

    fn connect(&self, autostart: Option<bool>) -> anyhow::Result<Client> {
        match &self.on {
            Some(machine) if !self.linked => {
                let control = self.paths.socket.parent().unwrap_or(Path::new("."));
                Ok(machines::connect(
                    machine,
                    control,
                    autostart == Some(true),
                )?)
            }
            _ => connect(self.paths, self.dir, autostart),
        }
    }
}

fn links(paths: &Paths) -> BTreeMap<String, Link> {
    let links = Client::connect(&paths.socket)
        .and_then(|mut c| c.call(Method::MachineLinks, &serde_json::json!({})));
    links.unwrap_or_default()
}

#[derive(Args)]
struct RunArgs {
    #[arg(long)]
    name: Option<String>,
    #[arg(long = "label", value_name = "K=V", value_parser = label)]
    labels: Vec<(String, String)>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    keep: bool,
    #[arg(long)]
    autostart: bool,
    #[arg(long, value_name = "S")]
    idle_after: Option<u64>,
    #[arg(long, value_name = "ID", conflicts_with_all = ["keep", "idle_after"])]
    acp: Option<String>,
    #[arg(long, requires = "acp", value_parser = policy, default_value = "ask")]
    policy: Policy,
    #[arg(long, value_name = "ID", requires = "acp")]
    resume: Option<String>,
    #[arg(long, value_name = "FILE", requires = "acp")]
    mcp: Option<PathBuf>,
    #[arg(long, value_name = "JSON", requires = "acp", value_parser = |s: &str| serde_json::from_str::<Map<String, Value>>(s))]
    meta: Option<Map<String, Value>>,
    #[arg(long, requires = "acp")]
    no_hibernate: bool,
    #[command(flatten)]
    parent: ParentArgs,
    #[arg(long, value_name = "RxC", value_parser = size)]
    size: Option<(u16, u16)>,
    #[arg(last = true, required_unless_present = "acp")]
    cmd: Vec<String>,
}

#[derive(Args)]
struct ParentArgs {
    #[arg(long, value_name = "T")]
    parent: Option<String>,
    #[arg(long, conflicts_with = "parent")]
    no_parent: bool,
    #[arg(long)]
    outlive_parent: bool,
    #[arg(long)]
    notify: bool,
}

#[derive(Args)]
struct ExecArgs {
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long, value_name = "S")]
    timeout: Option<f64>,
    #[arg(short = 'i')]
    stdin: bool,
    #[arg(last = true, required = true)]
    cmd: Vec<String>,
}

#[derive(Args)]
struct WaitArgs {
    #[arg(required_unless_present = "children")]
    name: Option<String>,
    #[arg(long, value_name = "all|any", value_parser = children, conflicts_with_all = ["until", "output"])]
    children: Option<Children>,
    #[arg(long, num_args = 1.., value_name = "STATE",
          value_parser = ["working", "blocked", "done", "error", "idle", "hibernated"])]
    until: Vec<String>,
    #[arg(long, value_name = "RE")]
    output: Option<String>,
    #[arg(long, value_name = "S")]
    timeout: Option<f64>,
}

fn policy(s: &str) -> Result<Policy, String> {
    serde_json::from_value(Value::String(s.to_owned()))
        .map_err(|_| format!("{s:?}: allow, deny or ask"))
}

fn children(s: &str) -> Result<Children, String> {
    serde_json::from_value(Value::String(s.to_owned())).map_err(|_| format!("{s:?}: all or any"))
}

fn size(s: &str) -> Result<(u16, u16), String> {
    shed_server::config::size(s).ok_or_else(|| format!("{s:?} is not RxC, e.g. 50x200"))
}

fn label(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| format!("{s:?} is not k=v"))
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            out::err(format_args!("shed: {e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(mut cli: Cli) -> anyhow::Result<ExitCode> {
    let paths = match &cli.dir {
        Some(dir) => Paths::in_dir(dir),
        None => Paths::user()?,
    };
    let json = cli.json;
    let server = Server::new(&paths, cli.dir.as_deref(), cli.on, &mut cli.command)?;
    match cli.command {
        Command::Hello => out::line(format_args!("shed {}", env!("CARGO_PKG_VERSION"))),
        Command::Schema => out::line(format_args!("{:#}", shed_server::proto::openrpc())),
        Command::Serve { detach, sample } => serve(&paths, cli.dir.as_deref(), detach, sample)?,
        Command::Run(args) => run_terminal(&server, json, *args)?,
        Command::Ls { labels, parent } => {
            let labels = labels.into_iter().collect();
            ls(&server, json, &LsParams { labels, parent })?;
        }
        Command::Get { name } => {
            let info: TerminalInfo = server.connect(None)?.call(Method::Get, &Target { name })?;
            let pretty = serde_json::to_string_pretty(&info)?;
            print(json, &info, || format!("{pretty}\n"))?;
        }
        Command::Read {
            name,
            lines,
            history,
            events,
            before,
            ..
        } => {
            let params = ReadParams {
                name,
                lines,
                history,
                events,
                before,
                n: lines.filter(|_| events),
            };
            read(&server, json, &params)?;
        }
        Command::Send { name, text } => send(&server, json, name, text)?,
        Command::Approve { name, option } => to_acp(&server, json, name, &Input::Approve(option))?,
        Command::Cancel { name } => to_acp(&server, json, name, &Input::Cancel)?,
        Command::Model { name, model } => {
            let params = ModelParams { name, model };
            let done: Value = server.connect(None)?.call(Method::Model, &params)?;
            print(json, &done, String::new)?;
        }
        Command::Agents { action } => agents_cmd(&paths, json, action)?,
        Command::AcpBridge {
            socket,
            state,
            policy,
            agent,
            cmd,
        } => return bridge(&cmd, &socket, &state, policy, &agent),
        Command::Keys { name, keys } => {
            let done: Value = server
                .connect(None)?
                .call(Method::Keys, &KeysParams { name, keys })?;
            print(json, &done, String::new)?;
        }
        Command::Kill { name } => {
            let info: TerminalInfo = server.connect(None)?.call(Method::Kill, &Target { name })?;
            print(json, &info, || match info.exit {
                Some(code) => format!("{} exited {code}\n", info.name),
                None => format!("{} exited\n", info.name),
            })?;
        }
        Command::Rm { name } => {
            let info: TerminalInfo = server.connect(None)?.call(Method::Rm, &Target { name })?;
            print(json, &info, || format!("{} removed\n", info.name))?;
        }
        Command::Attach { name } => attach::attach(server.connect(None)?, &name)?,
        Command::State { args, note } => state(&server, json, args, note)?,
        Command::Wait(args) => return wait(&server, json, args),
        Command::Events { labels, since } => {
            let params = EventsParams {
                labels: labels.into_iter().collect(),
                since,
            };
            events(&server, &params)?;
        }
        Command::Top => top(&server, json)?,
        Command::Watch { labels } => watch::watch(&server, labels.into_iter().collect())?,
        Command::Connect { autostart } => relay(&paths, cli.dir.as_deref(), autostart)?,
        Command::Machine(command) => return machine(&server, json, command),
        Command::Exec(args) => return exec(&server, json, args),
        Command::File { action, path } => file(&server, json, &action, path)?,
        Command::Install { force } => service::install(force)?,
        Command::Uninstall => service::uninstall()?,
        Command::Doctor if !service::doctor(&paths)? => return Ok(ExitCode::FAILURE),
        Command::Doctor => {}
    }
    Ok(ExitCode::SUCCESS)
}

fn ls(server: &Server, json: bool, params: &LsParams) -> anyhow::Result<()> {
    let mut list: Vec<TerminalInfo> = server.connect(None)?.call(Method::Ls, params)?;
    if server.linked {
        let prefix = server.name(String::new());
        list.retain(|t| t.name.starts_with(&prefix));
    }
    print(json, &list, || table(&list))
}

fn serve(paths: &Paths, dir: Option<&Path>, detach: bool, sample: f64) -> anyhow::Result<()> {
    if detach {
        return serve_detached(paths, dir, Some(sample));
    }
    let every = Duration::try_from_secs_f64(sample)
        .ok()
        .filter(|d| !d.is_zero())
        .context("--sample must be a positive number of seconds")?;
    shed_server::serve(paths, every).context("shed serve")
}

fn run_terminal(server: &Server, json: bool, args: RunArgs) -> anyhow::Result<()> {
    let cwd = match (&server.on, args.cwd) {
        (None, cwd) => {
            let here = std::env::current_dir().context("current directory")?;
            cwd.map_or_else(|| here.clone(), |d| here.join(d))
        }
        (Some(_), Some(d)) if d.is_absolute() => d,
        (Some(m), _) => bail!("with --on, pass --cwd as an absolute path on {}", m.name),
    };
    let mcp_servers = match &args.mcp {
        Some(file) => serde_json::from_slice(&std::fs::read(file)?)
            .with_context(|| format!("{}: not a JSON list", file.display()))?,
        None => Vec::new(),
    };
    let params = RunParams {
        name: args.name,
        labels: args.labels.into_iter().collect(),
        cwd,
        keep: args.keep,
        idle_after: args.idle_after,
        cmd: args.cmd,
        policy: args.acp.is_some().then_some(args.policy),
        acp: args.acp,
        resume: args.resume,
        mcp_servers,
        meta: args.meta,
        no_hibernate: args.no_hibernate,
        parent: match (args.parent.no_parent, &server.on) {
            (true, _) => None,
            (false, None) => (args.parent.parent).or_else(|| shed_server::config::var("SHED")),
            (false, Some(_)) => args.parent.parent,
        }
        .map(|name| Parent {
            name,
            outlive: args.parent.outlive_parent,
            notify: args.parent.notify,
        }),
        machine: server.machine(),
        rows: args.size.map(|s| s.0),
        cols: args.size.map(|s| s.1),
    };
    let mut client = server.connect(Some(args.autostart))?;
    let info: TerminalInfo = client.call(Method::Run, &params)?;
    print(json, &info, || format!("{}\n", info.name))
}

fn relay(paths: &Paths, dir: Option<&Path>, autostart: bool) -> anyhow::Result<()> {
    if UnixStream::connect(&paths.socket).is_err() {
        if !autostart {
            bail!(
                "no shed server on {}; start one with `shed serve --detach`",
                paths.socket.display()
            );
        }
        serve_detached(paths, dir, None)?;
    }
    shed_server::relay(&paths.socket).context("shed connect")
}

fn read(server: &Server, json: bool, params: &ReadParams) -> anyhow::Result<()> {
    let mut read: ReadResult = server.connect(None)?.call(Method::Read, params)?;
    if params.events && !json {
        let mut preview = shed_server::acp::Preview::default();
        for e in read.events.drain(..) {
            preview.push(e.kind);
        }
        let (cols, _, color) = shed_server::acp::stdout_style();
        read.text = preview.draw(cols, color, usize::MAX).join("\n");
    }
    let end = if read.text.is_empty() { "" } else { "\n" };
    print(json, &read, || format!("{}{end}", read.text))
}

fn send(server: &Server, json: bool, name: String, text: String) -> anyhow::Result<()> {
    let mut client = server.connect(None)?;
    let info: TerminalInfo = client.call(Method::Get, &Target { name: name.clone() })?;
    let text = if info.acp.is_some() {
        let report = Reported {
            state: "working".to_owned(),
            note: None,
            state_source: None,
        };
        let params = StateParams {
            name: name.clone(),
            report,
        };
        let _: TerminalInfo = client.call(Method::State, &params)?;
        acp::frame(&Input::Prompt(text))
    } else {
        text
    };
    let done: Value = client.call(Method::Send, &SendParams { name, text })?;
    print(json, &done, String::new)
}

fn bridge(
    cmd: &[String],
    socket: &Path,
    state: &Path,
    policy: Policy,
    agent: &str,
) -> anyhow::Result<ExitCode> {
    let here = std::env::current_dir().context("current directory")?;
    let _raw = attach::RawMode::enter(1, 0)?;
    let code = acp::bridge(cmd, &here, socket, state, policy, agent)?;
    Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)))
}

fn to_acp(server: &Server, json: bool, name: String, input: &Input) -> anyhow::Result<()> {
    let mut client = server.connect(None)?;
    let info: TerminalInfo = client.call(Method::Get, &Target { name: name.clone() })?;
    if info.acp.is_none() {
        bail!("{name} is not an ACP terminal (shed run --acp)");
    }
    let text = acp::frame(input);
    let done: Value = client.call(Method::Send, &SendParams { name, text })?;
    print(json, &done, String::new)
}

fn agents_cmd(paths: &Paths, json: bool, action: Option<AgentsCommand>) -> anyhow::Result<()> {
    let line = |a: &agents::Agent| format!("{} {} ({})\n", a.id, a.version, a.distribution);
    match action {
        None => {
            let list = agents::list(&paths.state)?;
            print(json, &list, || {
                list.iter()
                    .map(|a| format!("{}  {}", line(a).trim_end(), a.cmd.join(" ")) + "\n")
                    .collect()
            })
        }
        Some(AgentsCommand::Add { id, registry }) => {
            let agent = agents::add(&paths.state, &id, &registry)?;
            print(json, &agent, || format!("installed {}", line(&agent)))
        }
        Some(AgentsCommand::Rm { id }) => {
            let agent = agents::rm(&paths.state, &id)?;
            print(json, &agent, || format!("removed {}", line(&agent)))
        }
    }
}

fn top(server: &Server, json: bool) -> anyhow::Result<()> {
    let mut list: Vec<TerminalInfo> = server
        .connect(None)?
        .call(Method::Ls, &LsParams::default())?;
    list.sort_by_key(|t| std::cmp::Reverse(t.resources.map(|r| r.rss_mb)));
    print(json, &list, || table(&list))
}

fn state(
    server: &Server,
    json: bool,
    mut args: Vec<String>,
    note: Option<String>,
) -> anyhow::Result<()> {
    let state = args.pop().unwrap_or_default();
    let name = match args.pop() {
        Some(name) => name,
        None => shed_server::config::var("SHED")
            .context("no terminal given and $SHED is unset (it is set inside a terminal)")?,
    };
    let params = StateParams {
        name: server.name(name),
        report: Reported {
            state,
            note,
            state_source: None,
        },
    };
    let info: TerminalInfo = server.connect(None)?.call(Method::State, &params)?;
    print(json, &info, String::new)
}

fn wait(server: &Server, json: bool, args: WaitArgs) -> anyhow::Result<ExitCode> {
    let name = match args.name {
        Some(name) => name,
        None => server.name(
            shed_server::config::var("SHED")
                .context("no terminal given and $SHED is unset (it is set inside a terminal)")?,
        ),
    };
    let params = WaitParams {
        name,
        children: args.children,
        until: args.until,
        output: args.output,
        timeout: args.timeout,
    };
    let done: WaitResult = server.connect(None)?.call(Method::Wait, &params)?;
    let t = &done.terminal;
    let state = t.reported.as_ref().map(|r| &r.state);
    print(json, &done, || match (done.outcome, t.exit) {
        (WaitOutcome::Matched, _) if params.children == Some(Children::All) => {
            format!("{} no child working\n", t.name)
        }
        (WaitOutcome::Matched, code) if params.children.is_some() && t.state == State::Exited => {
            format!(
                "{} exited{}\n",
                t.name,
                code.map(|c| format!(" {c}")).unwrap_or_default()
            )
        }
        (WaitOutcome::Matched, _) if params.children.is_some() => {
            format!("{} {}\n", t.name, state.map_or("running", String::as_str))
        }
        (WaitOutcome::Matched, _) => match state.filter(|s| params.until.contains(s)) {
            Some(state) => format!("{} {state}\n", t.name),
            None => format!("{} output\n", t.name),
        },
        (WaitOutcome::Exited, Some(code)) => format!("{} exited {code}\n", t.name),
        (WaitOutcome::Exited, None) => format!("{} exited\n", t.name),
        (WaitOutcome::Timeout, _) => format!("{} timeout\n", t.name),
    })?;
    Ok(ExitCode::from(match done.outcome {
        WaitOutcome::Matched => 0,
        WaitOutcome::Exited => 1,
        WaitOutcome::Timeout => 124,
    }))
}

fn events(server: &Server, params: &EventsParams) -> anyhow::Result<()> {
    let mut client = server.connect(None)?;
    let _: Value = client.call(Method::Events, params)?;
    let mut out = io::stdout().lock();
    for line in client.split().0.lines() {
        let mut line = line?;
        if let Some(m) = &server.on
            && let Ok(mut event) = serde_json::from_str::<Event>(&line)
        {
            let prefix = format!("{}/", m.name);
            if server.linked && !event.name.starts_with(&prefix) {
                continue;
            }
            if !server.linked {
                event.name.insert_str(0, &prefix);
                line = serde_json::to_string(&event)?;
            }
        }
        writeln!(out, "{line}")?;
        out.flush()?;
    }
    Ok(())
}

fn machine(server: &Server, json: bool, command: MachineCommand) -> anyhow::Result<ExitCode> {
    let paths = server.paths;
    match command {
        MachineCommand::Add { name, target } => {
            let m = machines::add(&paths.config, &name, &target)?;
            links(paths);
            print(json, &m, || format!("{} added\n", m.name))
        }
        MachineCommand::Ls => {
            let links = links(paths);
            let list: Vec<Listed> = machines::load(&paths.config)?
                .into_iter()
                .map(|machine| Listed {
                    link: links.get(&machine.name).cloned(),
                    machine,
                })
                .collect();
            print(json, &list, || {
                let width = list.iter().map(|l| l.machine.name.len()).max();
                let width = width.unwrap_or(0).max(4);
                let mut out = format!("{:<width$}  TARGET\n", "NAME");
                for Listed { machine: m, link } in &list {
                    let link = link.as_ref().map_or_else(String::new, |l| {
                        format!("  {:?} since {}", l.state, l.since).to_lowercase()
                    });
                    writeln!(out, "{:<width$}  {}{link}", m.name, m.target).ok();
                }
                out
            })
        }
        MachineCommand::Rm { name } => {
            let m = machines::remove(&paths.config, &name)?;
            print(json, &m, || format!("{} removed\n", m.name))
        }
        MachineCommand::Stats { since } => {
            return machine_stats(server, json, StatsParams { since });
        }
    }?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    machine: Machine,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<Link>,
}

fn machine_stats(server: &Server, json: bool, params: StatsParams) -> anyhow::Result<ExitCode> {
    let stats: MachineStats = match &server.on {
        None => server.connect(None)?.call(Method::MachineStats, &params)?,
        Some(m) => {
            let control = server.paths.socket.parent().unwrap_or(Path::new("."));
            let seen = server.paths.state.with_file_name("seen");
            match machines::stats(m, control, &seen, params)? {
                Ok(stats) => stats,
                Err(offline) => {
                    print(json, &offline, || match offline.last_seen {
                        Some(t) => format!("{} offline, last seen {t}\n", m.name),
                        None => format!("{} offline, never seen\n", m.name),
                    })?;
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
    };
    let s = &stats.now;
    let terminals: Vec<String> = s
        .terminals
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect();
    print(json, &stats, || {
        let (load, mem, swap, disk) = (
            s.load_per_cpu,
            s.mem_pressure_pct,
            s.swap_used_mb,
            s.disk_free_mb,
        );
        let terminals = terminals.join(",");
        format!(
            "load/cpu {load:.2}  memory {mem}%  swap {swap}M  disk free {disk}M  cpus {}  up {}s  terminals {terminals}\n",
            s.cpus, s.uptime_s
        )
    })?;
    Ok(ExitCode::SUCCESS)
}

fn print<T: Serialize>(json: bool, value: &T, text: impl FnOnce() -> String) -> anyhow::Result<()> {
    if json {
        out::line(serde_json::to_string(value)?);
    } else {
        out::text(text());
    }
    Ok(())
}

fn exec(server: &Server, json: bool, args: ExecArgs) -> anyhow::Result<ExitCode> {
    let here = server.on.is_none().then(PathBuf::new);
    let params = ExecParams {
        cwd: args.cwd.or(here).map(|d| server.resolve(d)).transpose()?,
        timeout: args.timeout,
        stdin: args
            .stdin
            .then(stdin_bytes)
            .transpose()?
            .unwrap_or_default(),
        machine: server.machine(),
        cmd: args.cmd,
    };
    let done: ExecResult = server.connect(None)?.call(Method::Exec, &params)?;
    raw(json, &done, &done.stdout, &done.stderr)?;
    Ok(ExitCode::from(u8::try_from(done.code).unwrap_or(1)))
}

fn file(server: &Server, json: bool, action: &str, path: PathBuf) -> anyhow::Result<()> {
    let params = FileParams {
        path: server.resolve(path)?,
        data: (action == "write").then(stdin_bytes).transpose()?,
        machine: server.machine(),
    };
    let method = match params.data {
        Some(_) => Method::FileWrite,
        None => Method::FileRead,
    };
    let done: Value = server.connect(None)?.call(method, &params)?;
    let bytes: Vec<u8> = serde_json::from_value(done.clone()).unwrap_or_default();
    raw(json, &done, &bytes, &[])
}

fn stdin_bytes() -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes).context("stdin")?;
    Ok(bytes)
}

fn raw<T: Serialize>(json: bool, value: &T, stdout: &[u8], stderr: &[u8]) -> anyhow::Result<()> {
    if json {
        out::line(serde_json::to_string(value)?);
        return Ok(());
    }
    io::stdout().write_all(stdout)?;
    io::stdout().flush()?;
    io::stderr().write_all(stderr)?;
    Ok(())
}

fn connect(paths: &Paths, dir: Option<&Path>, autostart: Option<bool>) -> anyhow::Result<Client> {
    match Client::connect(&paths.socket) {
        Err(shed_server::Error::Io(e))
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            if autostart == Some(true) {
                let started = serve_detached(paths, dir, None);
                for _ in 0..50 {
                    if let Ok(client) = Client::connect(&paths.socket) {
                        return Ok(client);
                    }
                    shed_server::clock::sleep(Duration::from_millis(100));
                }
                started?;
                return Ok(Client::connect(&paths.socket)?);
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
        client => Ok(client?),
    }
}

fn serve_detached(paths: &Paths, dir: Option<&Path>, sample: Option<f64>) -> anyhow::Result<()> {
    if UnixStream::connect(&paths.socket).is_ok() {
        bail!("a shed server already runs on {}", paths.socket.display());
    }
    let mut args = vec![OsString::from("serve")];
    if let Some(dir) = dir {
        args.extend(["--dir".into(), dir.into()]);
    }
    if let Some(sample) = sample {
        args.extend(["--sample".into(), sample.to_string().into()]);
    }
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
                ready.replacen(' ', " on ", 1)
            )),
            None => bail!("shed serve exited ({}) before it was ready", child.wait()?),
        },
        Err(_) => bail!("shed serve was not ready within 10 s (still running, its stdout silent)"),
    }
    Ok(())
}

fn table(list: &[TerminalInfo]) -> String {
    let dash = |v: Option<String>| v.unwrap_or_else(|| "-".to_owned());
    let mut rows = vec![
        [
            "NAME", "STATE", "EXIT", "PID", "RSS", "CPU", "LABELS", "CMD",
        ]
        .map(str::to_owned),
    ];
    for t in list {
        let labels: Vec<String> = t.labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        rows.push([
            t.name.clone(),
            match t.state {
                State::Running => "running",
                State::Exited => "exited",
            }
            .to_owned(),
            dash(t.exit.map(|v| v.to_string())),
            dash(t.pid.map(|v| v.to_string())),
            dash(t.resources.map(|r| format!("{}M", r.rss_mb))),
            dash(t.resources.map(|r| format!("{}%", r.cpu_pct))),
            if labels.is_empty() {
                "-".to_owned()
            } else {
                labels.join(",")
            },
            t.cmd.join(" "),
        ]);
    }
    let mut widths = [0; 8];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = cell.chars().count().max(*w);
        }
    }
    let mut out = String::new();
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect();
        writeln!(out, "{}", cells.join("  ").trim_end()).ok();
    }
    out
}
