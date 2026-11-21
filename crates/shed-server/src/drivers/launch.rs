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

pub(crate) const LANE_ONLY: [&str; 4] = [
    "SHED_ACP_RESUME",
    "SHED_ACP_MCP",
    "SHED_ACP_META",
    "SHED_ACP_HIBERNATE_AFTER",
];

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
    if p.no_hibernate {
        set("SHED_ACP_HIBERNATE_AFTER", "0".to_owned());
    }
    Ok(vars)
}
