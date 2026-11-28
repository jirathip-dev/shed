use std::path::Path;

use shed_proto::{DriverKind, ErrorCode, Policy, RpcError, RunParams};

fn bad(what: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::InvalidParams, what)
}

pub(crate) fn bridge(
    p: &mut RunParams,
    socket: &Path,
    state: &Path,
    providers: &Path,
) -> Result<(), RpcError> {
    let internal = |e: String| RpcError::new(ErrorCode::Internal, e);
    let cmd = std::mem::take(&mut p.cmd);
    let id = p.acp.as_deref().unwrap_or_default();
    let policy = match p.policy.unwrap_or(Policy::Ask) {
        Policy::Allow => "allow",
        Policy::Deny => "deny",
        Policy::Ask => "ask",
        Policy::Judge if p.judge.is_empty() => {
            return Err(bad("policy judge needs a judge command"));
        }
        Policy::Judge => "judge",
    };
    let installed = crate::agents::list(state).map_err(|e| internal(e.to_string()))?;
    let adapter = installed
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.cmd.as_slice());
    let catalog = crate::providers::Catalog::load(providers).0;
    let record = catalog.by_id(id).cloned().unwrap_or_default();
    let chosen = crate::drivers::choose(&record, policy, &cmd, adapter, p.meta.as_ref());
    let kind = if chosen.is_ok() {
        DriverKind::Stream
    } else {
        DriverKind::Acp
    };
    let picked = (record.pick(kind, p.model.as_deref(), p.effort.as_deref())).map_err(bad)?;
    let driver = match &chosen {
        Ok(_) => vec!["--stream".to_owned()],
        Err(why) if record.driver == DriverKind::Stream => vec![format!("--why={why}")],
        Err(_) => Vec::new(),
    };
    let cmd = match (chosen, adapter) {
        (Ok(stream), _) => stream,
        (Err(_), _) if !cmd.is_empty() => cmd,
        (Err(_), Some(adapter)) => adapter.to_vec(),
        (Err(_), None) => {
            let ids: Vec<_> = installed.iter().map(|a| a.id.as_str()).collect();
            let ids = if ids.is_empty() {
                "none".to_owned()
            } else {
                ids.join(", ")
            };
            return Err(bad(format!(
                "ACP agent {id} is not installed; installed: {ids}; `shed agents add {id}`"
            )));
        }
    };
    p.env.extend(picked.env);
    if let (Some(var), true) = (&record.caller_var, record.caller) {
        let named = record.env.get(var).map_or("shed", String::as_str);
        p.env.entry(var.clone()).or_insert_with(|| named.to_owned());
    }
    let exe = std::env::current_exe().map_err(|e| internal(e.to_string()))?;
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let bridge = [
        path(&exe),
        "acp-bridge".to_owned(),
        "--socket".to_owned(),
        path(socket),
        "--state".to_owned(),
        path(state),
        "--policy".to_owned(),
        policy.to_owned(),
        "--agent".to_owned(),
        id.to_owned(),
        "--providers".to_owned(),
        path(providers),
    ];
    p.cmd = [
        bridge.to_vec(),
        driver,
        vec!["--".to_owned()],
        cmd,
        picked.args,
    ]
    .concat();
    Ok(())
}

pub(crate) const AUTO_ACCOUNT: &str = "SHED_ACP_AUTO";

pub(crate) const JUDGE: &str = "SHED_ACP_JUDGE";

pub(crate) const LANE_ONLY: [&str; 7] = [
    AUTO_ACCOUNT,
    crate::sandbox::WRAP_VAR,
    JUDGE,
    "SHED_ACP_RESUME",
    "SHED_ACP_MCP",
    "SHED_ACP_META",
    "SHED_ACP_HIBERNATE_AFTER",
];

pub(crate) fn agent<A: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: impl IntoIterator<Item = A>,
    cwd: &Path,
    account: &shed_accounts::env::Injected,
) -> std::io::Result<tokio::process::Child> {
    let unset = account.unset.iter().map(String::as_str);
    let unset: Vec<_> = LANE_ONLY.into_iter().chain(unset).collect();
    let wrap = crate::config::var(crate::sandbox::WRAP_VAR);
    let wrap: Vec<String> = wrap
        .and_then(|w| serde_json::from_str(&w).ok())
        .unwrap_or_default();
    let args = args
        .into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned());
    let mut cmd = wrap.into_iter().chain([program.to_owned()]).chain(args);
    let program = cmd.next().unwrap_or_default();
    shed_core::runner::piped(&program, cmd, cwd, (&unset, &account.set))
}

pub(crate) fn vars(p: &RunParams, name: &str) -> serde_json::Result<Vec<(String, String)>> {
    let mut vars: Vec<_> = p.env.clone().into_iter().collect();
    vars.push(("SHED".to_owned(), name.to_owned()));
    if p.acp.is_none() {
        return Ok(vars);
    }
    let mut set = |key: &str, value: String| vars.push((key.to_owned(), value));
    if let Some(id) = &p.resume {
        set("SHED_ACP_RESUME", id.clone());
    }
    if !p.mcp_servers.is_empty() {
        set("SHED_ACP_MCP", serde_json::to_string(&p.mcp_servers)?);
    }
    if let Some(meta) = &p.meta {
        set("SHED_ACP_META", serde_json::to_string(meta)?);
    }
    if p.policy == Some(Policy::Judge) {
        let judge = serde_json::json!({"cmd": p.judge, "secs": p.judge_secs, "labels": p.labels});
        set(JUDGE, judge.to_string());
    }
    if p.no_hibernate {
        set("SHED_ACP_HIBERNATE_AFTER", "0".to_owned());
    }
    Ok(vars)
}
