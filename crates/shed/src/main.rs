//! The `shed` binary: argument parsing and wiring only.
#![forbid(unsafe_code)]

mod attach;
mod service;
mod watch;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead as _, Write as _};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::Value;
use shed_server::acp::{self, Input, Policy};
use shed_server::agents;
use shed_server::machines::{self, Machine};
use shed_server::proto::{
    Event, EventsParams, KeysParams, Link, LinkState, LsParams, MachineStats, Method, ModelParams,
    ReadParams, ReadResult, Reported, RunParams, SendParams, State, StateParams, StatsParams,
    Target, TerminalInfo, WaitOutcome, WaitParams, WaitResult,
};
use shed_server::{Client, Paths};

/// Keeps coding-agent terminals alive, reads their screens and reports their state.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Use the socket (shed.sock) and state (state.json) in this directory instead of the per-user
    /// defaults.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Run the command on this machine (`shed machine add`) over SSH.
    #[arg(long, global = true, value_name = "MACHINE")]
    on: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the version.
    Hello,
    /// Run the per-user server: it owns the terminals, the state file and the socket.
    Serve {
        /// Start the server in the background and return once it answers.
        #[arg(long)]
        detach: bool,
        /// Sample each terminal's memory and CPU every S seconds.
        #[arg(long, value_name = "S", default_value_t = 2.0)]
        sample: f64,
    },
    /// Start a terminal; prints its name.
    Run(RunArgs),
    /// List terminals.
    Ls {
        /// Only terminals carrying this label, `k=v`; repeatable.
        #[arg(long = "label", value_name = "K=V", value_parser = label)]
        labels: Vec<(String, String)>,
    },
    /// One terminal as JSON.
    Get {
        /// The terminal.
        name: String,
    },
    /// The current screen, or the scrollback, as plain text.
    Read {
        /// The terminal.
        name: String,
        /// Only the last N lines (with `--events`, events).
        #[arg(long)]
        lines: Option<usize>,
        /// The visible screen (the default).
        #[arg(long, conflicts_with = "history")]
        screen: bool,
        /// The scrollback and the screen.
        #[arg(long)]
        history: bool,
        /// ACP: the transcript's events, drawn (`--json`: as they are, each with its `seq`).
        #[arg(long, conflicts_with_all = ["history", "screen"])]
        events: bool,
        /// With `--events`: only those before this `seq`.
        #[arg(long, requires = "events")]
        before: Option<u64>,
    },
    /// Type text; to an ACP terminal, send it as a prompt.
    Send {
        /// The terminal.
        name: String,
        /// The text, sent as is (add a newline, or `shed keys <t> enter`, to submit it).
        text: String,
    },
    /// Press keys: `enter`, `esc`, `ctrl+c`, `up`, …
    Keys {
        /// The terminal.
        name: String,
        /// Key names, pressed in order.
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// End the terminal's process tree.
    Kill {
        /// The terminal.
        name: String,
    },
    /// Remove an exited terminal.
    Rm {
        /// The terminal.
        name: String,
    },
    /// Take over the terminal interactively; `Ctrl+\` detaches.
    Attach {
        /// The terminal.
        name: String,
    },
    /// Report a terminal's state: `shed state [<t>] working|blocked|done|error` (default `$SHED`).
    State {
        /// The terminal, then the state; the terminal defaults to `$SHED`, set inside every terminal.
        #[arg(required = true, num_args = 1..=2, value_name = "[T] STATE")]
        args: Vec<String>,
        /// A free-text note.
        #[arg(long)]
        note: Option<String>,
    },
    /// Block until the reported state or the screen matches; exits 0 on a match, 1 when the program
    /// exits, 124 on timeout.
    Wait(WaitArgs),
    /// Stream events as JSON lines: started, state, exited, killed, resources, and ACP terminals' turn,
    /// text, `tool_call`, plan, mode, `approval_request`, approved and reply.
    Events {
        /// Only terminals carrying this label, `k=v`; repeatable.
        #[arg(long = "label", value_name = "K=V", value_parser = label)]
        labels: Vec<(String, String)>,
        /// First replay the kept events (the last 1,000) after this `seq`.
        #[arg(long, value_name = "N")]
        since: Option<u64>,
    },
    /// List terminals by memory use, largest first.
    Top,
    /// A live list of the terminals beside a read-only preview of the selected one; Enter attaches,
    /// `Ctrl+\` comes back, `/` filters, `q` quits.
    Watch {
        /// Only terminals carrying this label, `k=v`; repeatable.
        #[arg(long = "label", value_name = "K=V", value_parser = label)]
        labels: Vec<(String, String)>,
    },
    /// ACP: answer the oldest permission request with an option id (or kind, e.g. `allow_once`).
    Approve {
        /// The terminal.
        name: String,
        /// The option.
        option: String,
    },
    /// ACP: cancel the running turn (as `shed keys <t> esc`).
    Cancel {
        /// The terminal.
        name: String,
    },
    /// ACP: switch the session's model to one of `shed get`'s `session.models`.
    Model {
        /// The terminal.
        name: String,
        /// The model id.
        model: String,
    },
    /// List the installed ACP agents, or add or remove one.
    Agents {
        #[command(subcommand)]
        action: Option<AgentsCommand>,
    },
    /// The ACP bridge `run --acp` starts inside the terminal.
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
    /// Speak the protocol over stdin/stdout, for `ssh host shed connect`.
    Connect {
        /// Start `shed serve --detach` first when no server runs.
        #[arg(long)]
        autostart: bool,
    },
    /// Machines that `--on` reaches over SSH (key login only).
    #[command(subcommand)]
    Machine(MachineCommand),
    /// Write the launchd agent (macOS) or systemd user unit that runs `shed serve`; prints how to start it.
    Install {
        /// Replace a job file that has other contents.
        #[arg(long)]
        force: bool,
    },
    /// Print the lines that stop the job and remove its file.
    Uninstall,
    /// Check the job file, that it is loaded, the socket, the server's version and the log.
    Doctor,
}

impl Command {
    /// The terminal the command names, if it names one.
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
            | Self::Wait(WaitArgs { name, .. }) => Some(name),
            _ => None,
        }
    }
}

#[derive(Subcommand)]
enum AgentsCommand {
    /// Install an agent from the ACP registry, pinned to the version it lists now.
    Add {
        /// The registry id (`claude-acp`, `codex-acp`, `gemini`, `pi-acp`, …).
        id: String,
        /// The registry: a URL or a file.
        #[arg(long, value_name = "SRC", default_value = agents::REGISTRY)]
        registry: String,
    },
    /// Remove an installed agent.
    Rm {
        /// The agent id.
        id: String,
    },
}

/// `shed machine …`.
#[derive(Subcommand)]
enum MachineCommand {
    /// Add a machine: a name for an ssh target, `[user@]host`.
    Add {
        /// The name `--on` takes.
        name: String,
        /// The ssh target.
        target: String,
    },
    /// List the machines.
    Ls,
    /// Remove a machine.
    Rm {
        /// The machine.
        name: String,
    },
    /// This machine's health, or with `--on` another's (offline after 5 s without an answer, exit 1).
    Stats {
        /// Add the samples kept (one a minute, 24 h) after this time, unix seconds.
        #[arg(long, value_name = "T")]
        since: Option<u64>,
    },
}

/// Where the commands go: the local server, or the one on `--on`'s machine (over the local server's
/// held link when it is up, else one `ssh`).
struct Server<'a> {
    paths: &'a Paths,
    dir: Option<&'a Path>,
    on: Option<Machine>,
    linked: bool,
}

impl Server<'_> {
    /// The server `command` goes to: `--on`'s machine (refused for the local-only commands), else this
    /// one; over a held link the terminal it names becomes `<machine>/<name>`.
    fn new<'a>(
        paths: &'a Paths,
        dir: Option<&'a Path>,
        on: Option<String>,
        command: &mut Command,
    ) -> anyhow::Result<Server<'a>> {
        let local = matches!(
            command,
            Command::Hello
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

    /// `name` as the local server knows it: `<machine>/<name>` over a held link.
    fn name(&self, name: String) -> String {
        match &self.on {
            Some(m) if self.linked => format!("{}/{name}", m.name),
            _ => name,
        }
    }

    /// A client: over SSH with `--on` (`autostart` passed on), else the local one ([`connect`]).
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

/// The local server's held links (`machine.links`, which also links machines added since it
/// started); none when no server answers.
fn links(paths: &Paths) -> BTreeMap<String, Link> {
    let links = Client::connect(&paths.socket)
        .and_then(|mut c| c.call(Method::MachineLinks, &serde_json::json!({})));
    links.unwrap_or_default()
}

/// `shed run`'s arguments.
#[derive(Args)]
struct RunArgs {
    /// The terminal's name (default: the first free `tN`).
    #[arg(long)]
    name: Option<String>,
    /// A label, `k=v`; repeatable.
    #[arg(long = "label", value_name = "K=V", value_parser = label)]
    labels: Vec<(String, String)>,
    /// The working directory (default: the current one).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Come back as a shell in the same cwd after a server restart.
    #[arg(long)]
    keep: bool,
    /// Start `shed serve --detach` first when no server runs.
    #[arg(long)]
    autostart: bool,
    /// Report `idle` after S seconds without output, until the program reports a state.
    #[arg(long, value_name = "S")]
    idle_after: Option<u64>,
    /// Run the installed ACP agent ID (`shed agents`) as a structured session; with a command
    /// after `--`, that command is its ACP adapter instead.
    #[arg(long, value_name = "ID", conflicts_with_all = ["keep", "idle_after"])]
    acp: Option<String>,
    /// ACP: answer permission requests `allow`, `deny`, or `ask` (wait for `shed approve`).
    #[arg(long, requires = "acp", value_parser = policy, default_value = "ask")]
    policy: Policy,
    /// ACP: resume the agent's session ID (`shed get`'s `session.id`) instead of starting one.
    #[arg(long, value_name = "ID", requires = "acp")]
    resume: Option<String>,
    /// ACP: MCP servers for the agent, a JSON list in ACP's shapes (`docs/acp.md`); send it again
    /// with `--resume`.
    #[arg(long, value_name = "FILE", requires = "acp")]
    mcp: Option<PathBuf>,
    /// ACP: keep the agent running when idle instead of hibernating it.
    #[arg(long, requires = "acp")]
    no_hibernate: bool,
    /// The program and its arguments.
    #[arg(last = true, required_unless_present = "acp")]
    cmd: Vec<String>,
}

/// `shed wait`'s arguments.
#[derive(Args)]
struct WaitArgs {
    /// The terminal.
    name: String,
    /// States that end the wait.
    #[arg(long, num_args = 1.., value_name = "STATE",
          value_parser = ["working", "blocked", "done", "error", "idle", "hibernated"])]
    until: Vec<String>,
    /// A pattern (`.`, `[…]`, `\d \w \s`, `* + ?`, `^ $`) that ends the wait when a screen line matches.
    #[arg(long, value_name = "RE")]
    output: Option<String>,
    /// Give up after S seconds.
    #[arg(long, value_name = "S")]
    timeout: Option<f64>,
}

fn policy(s: &str) -> Result<Policy, String> {
    serde_json::from_value(Value::String(s.to_owned()))
        .map_err(|_| format!("{s:?}: allow, deny or ask"))
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
            eprintln!("shed: {e:#}");
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
        Command::Hello => println!("shed {}", env!("CARGO_PKG_VERSION")),
        Command::Serve { detach, sample } => serve(&paths, cli.dir.as_deref(), detach, sample)?,
        Command::Run(args) => run_terminal(&server, json, args)?,
        Command::Ls { labels } => {
            let params = LsParams {
                labels: labels.into_iter().collect(),
            };
            let mut list: Vec<TerminalInfo> = server.connect(None)?.call(Method::Ls, &params)?;
            if server.linked {
                let prefix = server.name(String::new());
                list.retain(|t| t.name.starts_with(&prefix));
            }
            print(json, &list, || table(&list))?;
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
            let n = lines.filter(|_| events);
            let params = ReadParams {
                name,
                lines,
                history,
                events,
                before,
                n,
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
        Command::Install { force } => service::install(force)?,
        Command::Uninstall => service::uninstall()?,
        Command::Doctor if !service::doctor(&paths)? => return Ok(ExitCode::FAILURE),
        Command::Doctor => {}
    }
    Ok(ExitCode::SUCCESS)
}

/// `shed serve [--detach] [--sample S]`.
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

/// `shed run`; with `--on` the cwd must be given, as an absolute path on the machine.
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
        no_hibernate: args.no_hibernate,
        machine: server
            .on
            .as_ref()
            .filter(|_| server.linked)
            .map(|m| m.name.clone()),
    };
    let mut client = server.connect(Some(args.autostart))?;
    let info: TerminalInfo = client.call(Method::Run, &params)?;
    print(json, &info, || format!("{}\n", info.name))
}

/// `shed connect [--autostart]`: the protocol over stdin/stdout.
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

/// `shed send`: an ACP terminal gets a framed prompt, reported `working` first so `send; wait --until
/// done` never sees the last turn.
/// `shed read`; with `--events` (but not `--json`) the events drawn as the preview.
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

/// `shed acp-bridge`: the bridge on this terminal, in raw mode; exits with the agent's code.
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

/// Delivers `input` to an ACP terminal's bridge.
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

/// `shed agents [add|rm]`.
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

/// `shed top`: the terminals by memory, largest first.
fn top(server: &Server, json: bool) -> anyhow::Result<()> {
    let mut list: Vec<TerminalInfo> = server
        .connect(None)?
        .call(Method::Ls, &LsParams::default())?;
    list.sort_by_key(|t| std::cmp::Reverse(t.resources.map(|r| r.rss_mb)));
    print(json, &list, || table(&list))
}

/// `shed state [<t>] <state>`: the terminal defaults to `$SHED`.
fn state(
    server: &Server,
    json: bool,
    mut args: Vec<String>,
    note: Option<String>,
) -> anyhow::Result<()> {
    let state = args.pop().unwrap_or_default();
    let name = match args.pop() {
        Some(name) => name,
        None => std::env::var("SHED")
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

/// `shed wait`: exit 0 on a match, 1 when the program exits, 124 on timeout.
fn wait(server: &Server, json: bool, args: WaitArgs) -> anyhow::Result<ExitCode> {
    let params = WaitParams {
        name: args.name,
        until: args.until,
        output: args.output,
        timeout: args.timeout,
    };
    let done: WaitResult = server.connect(None)?.call(Method::Wait, &params)?;
    let t = &done.terminal;
    let state = t.reported.as_ref().map(|r| &r.state);
    print(json, &done, || match (done.outcome, t.exit) {
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

/// `shed events`: copies the event lines to stdout until the server closes the connection. With
/// `--on`, only that machine's, named `<machine>/<name>`.
fn events(server: &Server, params: &EventsParams) -> anyhow::Result<()> {
    let mut client = server.connect(None)?;
    let _: Value = client.call(Method::Events, params)?;
    let mut out = io::stdout().lock();
    for line in client.split().0.lines() {
        let mut line = line?;
        if let Some(m) = &server.on {
            let mut event: Event = serde_json::from_str(&line)?;
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

/// `shed machine add|ls|rm|stats`.
fn machine(server: &Server, json: bool, command: MachineCommand) -> anyhow::Result<ExitCode> {
    let paths = server.paths;
    match command {
        MachineCommand::Add { name, target } => {
            let m = machines::add(&paths.config, &name, &target)?;
            // A running server links it now.
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
                    // Writing to a String cannot fail.
                    let _ = writeln!(out, "{:<width$}  {}{link}", m.name, m.target);
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

/// A configured machine, with its link when the local server holds one.
#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    machine: Machine,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<Link>,
}

/// `shed machine stats [--since T] [--on M]`: exit 1 when the machine is offline.
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

/// Prints `value` as one JSON line with `--json`, else `text()`.
fn print<T: Serialize>(json: bool, value: &T, text: impl FnOnce() -> String) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(value)?);
    } else {
        print!("{}", text());
    }
    Ok(())
}

/// Connects to the server. `autostart` is `Some` for `run`: with `true` a missing server is started
/// (`dir` passed on), with `false` the error offers `--autostart`.
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
                // A start running at the same time may have won: its server answers within 5 s.
                for _ in 0..50 {
                    if let Ok(client) = Client::connect(&paths.socket) {
                        return Ok(client);
                    }
                    std::thread::sleep(Duration::from_millis(100));
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

/// Starts `shed serve` in its own process group, detached from this terminal, and waits for its ready
/// line, racing it against the server exiting and a 10 s timeout.
fn serve_detached(paths: &Paths, dir: Option<&Path>, sample: Option<f64>) -> anyhow::Result<()> {
    if UnixStream::connect(&paths.socket).is_ok() {
        bail!("a shed server already runs on {}", paths.socket.display());
    }
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("serve");
    if let Some(dir) = dir {
        cmd.arg("--dir").arg(dir);
    }
    if let Some(sample) = sample {
        cmd.arg("--sample").arg(sample.to_string());
    }
    let mut child = cmd
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting shed serve")?;
    let stdout = child.stdout.take().context("shed serve stdout")?;
    let (tx, rx) = mpsc::channel();
    // The line, or an empty one at EOF: the server closed its stdout by exiting.
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = io::BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(line) => match line.trim_end().strip_prefix("shed ready ") {
            Some(ready) => eprintln!("shed: server {} started", ready.replacen(' ', " on ", 1)),
            None => bail!("shed serve exited ({}) before it was ready", child.wait()?),
        },
        Err(_) => bail!("shed serve was not ready within 10 s (still running, its stdout silent)"),
    }
    Ok(())
}

/// `ls` and `top` as an aligned table: name, state, exit, pid, tree memory and CPU, labels, command.
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
        // Writing to a String cannot fail.
        let _ = writeln!(out, "{}", cells.join("  ").trim_end());
    }
    out
}
