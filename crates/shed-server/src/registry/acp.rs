use super::{Registry, not_found};
use crate::input::Input;
use shed_accounts::store::DEFAULT;
use shed_proto::{
    AcpSession, EmitParams, ErrorCode, EventKind, ModelParams, Provider, RpcError, State,
};
use std::sync::Arc;
use std::time::Instant;

impl Registry {
    pub(crate) fn emit(&mut self, mut p: EmitParams) -> Result<(), RpcError> {
        if !p.kind.is_acp() && !matches!(p.kind, EventKind::Account { .. }) {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                "emit takes only ACP events",
            ));
        }
        p.name = self.named(&p.name);
        self.live(&p.name)?;
        let entry = self
            .entries
            .get_mut(&p.name)
            .ok_or_else(|| not_found(&p.name))?;
        if let EventKind::Turn { agent, .. } = &mut p.kind {
            let agent = std::mem::take(agent);
            if !p.replay {
                entry.agent = Some(agent.into_iter().collect());
            }
        }
        let now = shed_core::clock::now();
        let ms = |since: Instant| {
            let took = now.saturating_duration_since(since).as_millis();
            u64::try_from(took).unwrap_or(u64::MAX)
        };
        let info = &mut entry.info;
        let old = info.session.clone().unwrap_or_default();
        let mut timed = None;
        let session = match &p.kind {
            EventKind::Session(s) => {
                let ready = (!p.replay).then_some(entry.started).flatten();
                let new = AcpSession {
                    turn_ended: old.turn_ended,
                    ready_ms: old.ready_ms.or_else(|| ready.map(ms)),
                    first_token_ms: old.first_token_ms,
                    ..(**s).clone()
                };
                info.session
                    .replace(new.clone())
                    .is_none_or(|old| old != new)
            }
            EventKind::Reply { .. } if !p.replay => {
                info.session.get_or_insert_default().turn_ended =
                    Some(shed_core::clock::unix_now());
                true
            }
            EventKind::Account { account, .. } if !p.replay => {
                info.account = Some(account.clone());
                true
            }
            EventKind::Usage(u) => {
                info.session.get_or_insert_default().usage = Some(u.clone());
                false
            }
            EventKind::Turn { .. } if !p.replay => {
                entry.prompted = entry.prompted.or(Some(now));
                false
            }
            EventKind::Text { .. } if !p.replay && old.first_token_ms.is_none() => {
                let first = entry.prompted.map(ms);
                let session = info.session.get_or_insert_default();
                session.first_token_ms = first;
                timed = (info.acp.clone()).zip(old.ready_ms.zip(first));
                first.is_some()
            }
            _ => false,
        };
        self.events.emit_as(info, p.kind, p.replay);
        if let Some((agent, (ready, first))) = timed {
            crate::providers::record_timing(&self.path, &agent, ready, first).ok();
        }
        if session {
            self.save();
        }
        Ok(())
    }

    pub(crate) fn model(&self, p: ModelParams) -> Result<Input, RpcError> {
        let why = format!("{} is not an ACP terminal", p.name);
        let acp = self.entry(&p.name)?.info.acp.as_ref();
        acp.ok_or_else(|| RpcError::new(ErrorCode::InvalidParams, why))?;
        let text = shed_proto::frame(&shed_proto::Input::Model(p.model));
        Ok(Input::Now(
            Arc::clone(self.live(&p.name)?),
            text.into_bytes(),
        ))
    }

    pub(crate) fn use_account(
        &mut self,
        provider: &Provider,
        to: &str,
    ) -> Vec<(String, Result<Input, String>)> {
        let text = shed_proto::frame(&shed_proto::Input::Account(to.to_owned()));
        let mut told = Vec::new();
        for entry in self.entries.values_mut() {
            let info = &mut entry.info;
            let lane = info.acp.as_deref() == Some(provider.id.as_str());
            let pty = info.acp.is_none() && info.labels.get("agent") == Some(&provider.agent);
            let on = info.account.clone().unwrap_or_else(|| DEFAULT.to_owned());
            let why = match &entry.term {
                _ if info.state != State::Running || !(lane || pty) => continue,
                _ if pty => "not an ACP or stream lane: never switched".to_owned(),
                _ if provider.session_files.is_empty() => "no session files to copy".to_owned(),
                _ if on == to => format!("already on {to}"),
                _ if on != DEFAULT && !info.auto => format!("on named account {on}"),
                None => continue,
                Some(term) => {
                    info.auto = true;
                    let input = Input::Now(Arc::clone(term), text.clone().into_bytes());
                    told.push((info.name.clone(), Ok(input)));
                    continue;
                }
            };
            told.push((info.name.clone(), Err(why)));
        }
        self.save();
        told
    }
}
