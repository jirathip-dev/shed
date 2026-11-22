use std::sync::Mutex;
use std::time::Duration;

use shed_core::{STATES, Wake};
use shed_pattern::Pattern;

use crate::call::{Call, Reply, json};
use crate::proto::{
    Children, ErrorCode, Event, EventKind, EventsParams, RpcError, State, WaitOutcome, WaitParams,
    WaitResult,
};
use crate::registry::Registry;
use crate::{lane, lock};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    json(&wait(cx.registry, &cx.params()?).await?)
}

async fn wait(registry: &Mutex<Registry>, p: &WaitParams) -> Result<WaitResult, RpcError> {
    let bad = |e: &dyn std::fmt::Display| RpcError::new(ErrorCode::InvalidParams, e);
    if p.until.is_empty() && p.output.is_none() && p.children.is_none() {
        return Err(bad(&"give until, output or children"));
    }
    if let Some(s) = p.until.iter().find(|s| {
        !(*s == "idle" || STATES.contains(&s.as_str()) || lane::LANE_STATES.contains(&s.as_str()))
    }) {
        return Err(bad(&format!("unknown state {s:?}")));
    }
    if let Some(k) = p
        .kind
        .as_ref()
        .filter(|k| !["permission", "question", "auth"].contains(&k.as_str()))
    {
        return Err(bad(&format!("unknown kind {k:?}")));
    }
    let pattern = p.output.as_deref().map(Pattern::new).transpose();
    let pattern = pattern.map_err(|e| bad(&e))?;
    let timeout = p.timeout.map(Duration::try_from_secs_f64).transpose();
    let deadline = timeout
        .map_err(|e| bad(&e))?
        .and_then(|d| shed_core::clock::now().checked_add(d));
    if let Some(how) = p.children {
        return wait_children(registry, &p.name, how, deadline).await;
    }
    let (term, info) = {
        let mut reg = lock(registry);
        reg.refresh()?;
        reg.to_wait(&p.name)?
    };
    let reported = |state: &str, kind: Option<&str>| {
        p.until.iter().any(|s| s == state)
            && (state != "blocked" || p.kind.is_none() || p.kind.as_deref() == kind)
    };
    let outcome = match term {
        Some(term) => {
            let done = |t: &shed_core::Terminal| {
                t.report()
                    .is_some_and(|r| reported(&r.state, r.kind.as_deref()))
                    || pattern.as_ref().is_some_and(|re| {
                        let screen = t.screen();
                        let screen = if info.redacted {
                            shed_core::redact(&screen)
                        } else {
                            screen
                        };
                        screen.lines().any(|l| re.is_match(l))
                    })
            };
            match term
                .wait_until(done, deadline)
                .await
                .map_err(|e| RpcError::new(ErrorCode::Internal, e))?
            {
                Wake::Matched => WaitOutcome::Matched,
                Wake::Exited => WaitOutcome::Exited,
                Wake::Timeout => WaitOutcome::Timeout,
            }
        }
        None if info
            .reported
            .is_some_and(|r| reported(&r.state, r.kind.as_deref())) =>
        {
            WaitOutcome::Matched
        }
        None => WaitOutcome::Exited,
    };
    let mut reg = lock(registry);
    reg.refresh()?;
    Ok(WaitResult {
        outcome,
        terminal: reg.get(&p.name)?,
    })
}

async fn wait_children(
    registry: &Mutex<Registry>,
    name: &str,
    how: Children,
    deadline: Option<std::time::Instant>,
) -> Result<WaitResult, RpcError> {
    let mut sub = lock(registry).events.subscribe(EventsParams::default());
    let news = |e: &Event| {
        e.parent.as_deref() == Some(name)
            && match &e.kind {
                EventKind::State(r) => {
                    let news = [
                        "done",
                        "blocked",
                        "waiting-approval",
                        "rate-limited",
                        "error",
                    ];
                    news.contains(&r.state.as_str())
                }
                EventKind::Exited { .. } | EventKind::Killed => true,
                _ => false,
            }
    };
    loop {
        let child = sub.events().into_iter().find(news);
        {
            let mut reg = lock(registry);
            reg.refresh()?;
            let parent = reg.get(name)?;
            let running = || reg.children(name).any(|t| t.state == State::Running);
            let ended = match (how, child) {
                (Children::All, _) if !reg.busy_children(name) => {
                    Some((WaitOutcome::Matched, parent))
                }
                (Children::Any, Some(e)) => Some((WaitOutcome::Matched, reg.get(&e.name)?)),
                (Children::Any, None) if !running() => Some((WaitOutcome::Exited, parent)),
                _ => None,
            };
            if let Some((outcome, terminal)) = ended {
                return Ok(WaitResult { outcome, terminal });
            }
        }
        let changed = match deadline {
            Some(at) => tokio::time::timeout_at(at.into(), sub.changed()).await,
            None => Ok(sub.changed().await),
        };
        if !changed.unwrap_or(false) {
            let terminal = lock(registry).get(name)?;
            let outcome = WaitOutcome::Timeout;
            return Ok(WaitResult { outcome, terminal });
        }
    }
}
