use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::install::{self, asset};
use shed_server::proto::{BUILD, Handover, Method, Source, Stage, Swap};
use shed_server::runner;
use shed_server::{Paths, config};

fn main_head(repo: &str) -> anyhow::Result<String> {
    let ran = runner::output("git", ["ls-remote", repo, "refs/heads/main"], &[])?;
    let (said, err) = (
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr),
    );
    let sha = (said.lines()).find_map(|l| l.strip_suffix("\trefs/heads/main"));
    match sha.filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
        Some(sha) if ran.status.success() => Ok(sha.get(..7).unwrap_or_default().to_owned()),
        _ => bail!("git ls-remote {repo} main: {}", err.trim()),
    }
}

fn hand_over(serve: Option<Conn>, to: &str) -> String {
    match serve.map(|mut c| c.call::<_, Handover>(Method::Handover, &json!({"build": to}))) {
        None => "not running".to_owned(),
        Some(Ok(_)) => "handed over".to_owned(),
        Some(Err(e)) => format!("pending ({e}); the next pass asks again"),
    }
}

pub fn pass(paths: &Paths, dir: &Path, exe: &Path, dry: bool) -> anyhow::Result<String> {
    if let Ok(meta) = fs::metadata(dir.join("paused")) {
        let since = meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
        return Ok(format!("paused since {since}; shed self-update --resume"));
    }
    if !dry {
        crate::service::migrate_updater()?;
    }
    let (repo, releases) = config::update_source();
    let to = main_head(&repo)?;
    let tag = format!("build-{to}");
    if dry {
        crate::out::line(format_args!("target   {tag} (main)"));
    }
    let mut serve = Conn::connect(&paths.socket).ok();
    let serving = serve.as_ref().map(|c| c.server().build.clone());
    if to == BUILD {
        return Ok(match serving.filter(|b| b != BUILD) {
            None => format!("up to date on {BUILD}"),
            Some(b) if dry => format!("would    hand serve over from {b}"),
            Some(_) => format!("serve {} on {BUILD}", hand_over(serve, BUILD)),
        });
    }
    let source = Source {
        tool: "shed".to_owned(),
        releases,
        tag: tag.clone(),
    };
    if let Err(why) = install::find(&source) {
        return Ok(format!("waiting for {tag}: {why}"));
    }
    if dry {
        let asset = asset("shed");
        return Ok(format!("would    fetch {asset}, check, swap, hand over"));
    }
    let (dest, build) = (exe.to_owned(), to.clone());
    install::stage(&Stage {
        source,
        build,
        dest,
    })
    .context(tag.clone())?;
    let check = json!({"build": to, "candidate": exe.with_extension("new")});
    let checked = serve
        .as_mut()
        .map(|c| c.call::<_, Handover>(Method::HandoverCheck, &check));
    if let Some(Err(e)) = checked {
        return Ok(format!("deferred {tag}: {e}; staged, nothing swapped"));
    }
    let (dest, build) = (exe.to_owned(), to.clone());
    install::swap(&Swap { dest, build })?;
    let served = hand_over(serve, &to);
    Ok(format!("updated {BUILD} -> {to}; serve {served}"))
}

pub fn last(paths: &Paths) -> Option<String> {
    let text = fs::read(paths.state.with_file_name("update/status.json")).ok()?;
    let status: Value = serde_json::from_slice(&text).ok()?;
    status.get("why").and_then(Value::as_str).map(str::to_owned)
}
