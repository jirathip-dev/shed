//! The server: the socket, its terminals and the samplers.

use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustix::process::{Resource, Rlimit, Signal, getrlimit, setrlimit};
use shed_core::proctree::ProcessTable as _;
use shed_core::{Terminal, Wake};

use crate::call::blocking;
use crate::dispatch::connection;
use crate::proto::{ErrorCode, LONG_VERSION, RpcError, TerminalInfo};
use crate::registry::{self, Registry};
use crate::{Error, Paths, Shared, handover, links, lock, proctree, runner, stats};

pub(crate) fn watch(registry: &Shared, name: String, term: Arc<Terminal>) {
    let registry = Arc::clone(registry);
    tokio::spawn(async move {
        loop {
            let last = (term.report(), term.records());
            lock(&registry).refresh().ok();
            if !matches!(
                term.wait_until(|t| (t.report(), t.records()) != last, None)
                    .await,
                Ok(Wake::Matched)
            ) {
                let orphans = {
                    let mut reg = lock(&registry);
                    reg.refresh().ok();
                    reg.orphans(&reg.named(&name))
                };
                for child in orphans {
                    let registry = Arc::clone(&registry);
                    tokio::spawn(async move { kill(&registry, &child).await });
                }
                return;
            }
        }
    });
}

pub(crate) async fn kill(registry: &Shared, name: &str) -> Result<TerminalInfo, RpcError> {
    let term = lock(registry).mark_kill(name)?;
    let Some(term) = term else {
        return lock(registry).get(name);
    };
    let killing = Arc::clone(&term);
    let status = blocking(move || killing.kill(registry::GRACE)).await?;
    let status = status.map_err(|e| RpcError::new(ErrorCode::Internal, e))?;
    lock(registry).killed(name, &term, status)
}

async fn sample(registry: Shared, every: Duration) {
    loop {
        tokio::time::sleep(every).await;
        for name in lock(&registry).stale_spares() {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                kill(&registry, &name).await.ok();
                let files = lock(&registry).rm(&name).map(|(_, files)| files);
                if let Ok(files) = files {
                    blocking(files).await.ok();
                }
            });
        }
        if let Ok(Ok(procs)) = blocking(|| proctree::system().list()).await {
            for pid in lock(&registry).sample(&procs, shed_core::clock::now()) {
                let end =
                    move || proctree::kill_tree(&proctree::system(), pid, registry::GRACE, || {});
                tokio::spawn(blocking(end));
            }
        }
    }
}

async fn record_stats(registry: &Mutex<Registry>) -> Duration {
    let (wait, dir) = {
        let reg = lock(registry);
        (reg.stats.until_due(shed_core::clock::unix_now()), reg.dir())
    };
    if wait > 0 {
        return Duration::from_secs(wait);
    }
    let Ok(sample) = blocking(move || stats::system(&dir)).await else {
        return Duration::from_secs(1);
    };
    let save = lock(registry).record(sample);
    blocking(save).await.ok();
    let wait = lock(registry).stats.until_due(shed_core::clock::unix_now());
    Duration::from_secs(wait.max(1))
}

/// Loads the state, binds the socket (mode 0600, its directory 0700) or, with `inherit`, takes it and the
/// terminals from the image it replaced, prints `shed ready <semver> (<sha>) <socket>` and serves
/// until killed, sampling each terminal's memory and CPU every `sample_every`; `handover` restarts it as `again`.
///
/// # Errors
/// [`Error::AlreadyRunning`] when another server answers on the socket or holds the state file; [`Error::Io`] or
/// [`Error::Json`] when the state file or socket cannot be set up.
pub fn serve(
    paths: &Paths,
    sample_every: Duration,
    inherit: bool,
    again: handover::Restart,
) -> Result<(), Error> {
    let inherited = inherit.then(|| handover::take(io::stdin())).flatten();
    if inherited.is_none() && UnixStream::connect(&paths.socket).is_ok() {
        return Err(Error::AlreadyRunning(paths.socket.clone()));
    }
    if let Some(dir) = paths.state.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let held = fs::File::create(paths.state.with_extension("lock"))?;
    match held.try_lock() {
        Err(fs::TryLockError::WouldBlock) => {
            return Err(Error::AlreadyRunning(paths.socket.clone()));
        }
        locked => locked.map_err(io::Error::from)?,
    }
    raise_open_files();
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;
    let served = runtime.block_on(listen(paths, sample_every, inherited, again));
    runtime.shutdown_background();
    served
}

fn stop_signals() -> io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let skipped = [
        Signal::KILL,
        Signal::STOP,
        Signal::ILL,
        Signal::FPE,
        Signal::SEGV,
    ]
    .into_iter()
    .chain([Signal::BUS, Signal::TRAP, Signal::SYS])
    .map(Signal::as_raw)
    .collect::<Vec<_>>();
    for raw in (1..32).filter(|raw| !skipped.contains(raw)) {
        drop(signal(SignalKind::from_raw(raw))?);
    }
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    })
}

async fn listen(
    paths: &Paths,
    sample_every: Duration,
    inherited: Option<handover::Inherited>,
    again: handover::Restart,
) -> Result<(), Error> {
    let stop = stop_signals()?;
    let (listener, masters, early) = match inherited {
        Some(i) => (Some(i.listener), i.masters, i.early),
        None => (None, Vec::new(), Vec::new()),
    };
    let mut loaded = Registry::load(&paths.state, &paths.socket, masters)?;
    loaded.providers = paths.providers();
    let (asked, mut handovers) = tokio::sync::mpsc::unbounded_channel();
    loaded.control = Some(Arc::new(handover::Control { again, asked }));
    loaded.set_rules();
    let registry = Arc::new(Mutex::new(loaded));
    for (name, term) in lock(&registry).running() {
        watch(&registry, name, term);
    }
    tokio::spawn(registry::keep_saved(Arc::clone(&registry)));
    tokio::spawn(sample(Arc::clone(&registry), sample_every));
    tokio::spawn(crate::accounts::beat(Arc::clone(&registry)));
    let first = record_stats(&registry).await;
    let recorder = Arc::clone(&registry);
    tokio::spawn(async move {
        let mut wait = first;
        loop {
            tokio::time::sleep(wait).await;
            wait = record_stats(&recorder).await;
        }
    });
    let control = paths.socket.parent().unwrap_or(Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(control)?;
    let (links, watched) = links::Links::new(paths.config.clone(), control.to_owned());
    let links = Arc::new(links);
    links.sync(&registry);
    tokio::spawn(crate::flows::run(Arc::clone(&registry), Arc::clone(&links)));
    crate::proxy::start(
        &registry,
        &links,
        watched,
        bind(&crate::proxy::socket(control))?,
    );
    if let Err(e) = crate::inside::listen(&registry, &links, &paths.state) {
        writeln!(io::stderr(), "shed serve: no inside socket: {e}").ok();
    }
    let listener = match listener {
        Some(listener) => tokio::net::UnixListener::from_std(listener)?,
        None => bind(&paths.socket)?,
    };
    let early = early.into_iter().map(tokio::net::UnixStream::from_std);
    let early = early.collect::<io::Result<Vec<_>>>()?;
    writeln!(
        io::stdout(),
        "shed ready {LONG_VERSION} {}",
        paths.socket.display()
    )
    .ok();
    for early in early {
        tokio::spawn(connection(
            Arc::clone(&registry),
            Arc::clone(&links),
            early,
            false,
        ));
    }
    let mut stop = std::pin::pin!(stop);
    loop {
        tokio::select! {
            () = accept(&listener, &registry, &links, false) => {}
            () = &mut stop => break,
            Some(()) = handovers.recv() => {
                let (why, back) = handover::run(&registry, &listener).await;
                for (name, term) in back {
                    watch(&registry, name, term);
                }
                writeln!(io::stderr(), "shed serve: no handover: {why}").ok();
            }
        }
    }
    let saved = lock(&registry).close();
    saved.await;
    let rest = lock(&registry).release_lanes();
    let ends = rest
        .into_iter()
        .map(|t| blocking(move || t.kill(registry::GRACE)));
    futures::future::join_all(ends).await;
    Ok(())
}

pub(crate) fn bind(socket: &Path) -> io::Result<tokio::net::UnixListener> {
    match fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = tokio::net::UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub(crate) async fn accept(
    listener: &tokio::net::UnixListener,
    registry: &Shared,
    links: &Arc<links::Links>,
    inside: bool,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(connection(
                    Arc::clone(registry),
                    Arc::clone(links),
                    stream,
                    inside,
                ));
            }
            Err(e) => {
                writeln!(io::stderr(), "shed serve: accept: {e}").ok();
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

fn raise_open_files() {
    let old = getrlimit(Resource::Nofile);
    let cap = if cfg!(target_os = "macos") {
        runner::output("/usr/sbin/sysctl", ["-n", "kern.maxfilesperproc"], &[])
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok()?.trim().parse().ok())
    } else {
        None
    };
    let target = match (old.maximum, cap) {
        (Some(hard), Some(cap)) => Some(hard.min(cap)),
        (hard, None) => hard,
        (None, cap) => cap,
    };
    if (old.current).is_none_or(|soft| target.is_some_and(|target| soft >= target)) {
        return;
    }
    let new = Rlimit {
        current: target,
        ..old
    };
    if let Err(e) = setrlimit(Resource::Nofile, new) {
        let show = |n: Option<u64>| n.map_or_else(|| "unlimited".to_owned(), |n| n.to_string());
        let (from, to) = (show(old.current), show(new.current));
        writeln!(
            io::stderr(),
            "shed serve: open-file limit {from} -> {to}: {e}"
        )
        .ok();
    }
}
