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
        let mut spend = None;
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
                let was = info
                    .session
                    .get_or_insert_default()
                    .usage
                    .replace(u.clone());
                spend = (info.acp.clone())
                    .zip(info.account.clone())
                    .filter(|_| !p.replay)
                    .map(|on| (on, was, u.clone()));
                !p.replay
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
        if let Some(((acp, name), was, now)) = spend {
            let on = (acp.as_str(), name.as_str());
            crate::accounts::spent(self, on, was.as_ref(), &now, &crate::accounts::month()).ok();
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
            let on = info.account.clone().unwrap_or_else(|| DEFAULT.to_owned());
            let why = match &entry.term {
                _ if info.state != State::Running || !lane => continue,
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::{Value, json};
    use shed_accounts::Store;
    use shed_core::Terminal;
    use shed_proto::{AccountAdd, AcpUsage, EmitParams, EventKind, Tokens};

    use super::Registry;
    use crate::registry::tests::{dir, sleeper};

    const PRICED: &str = "id = \"key\"\nagent = \"key\"\ntoken_var = \"KEY_API_KEY\"\n[price]\n\
                          currency = \"USD\"\ninput = 2.0\ncache_read = 0.5\ncache_write = 1.0\noutput = 8.0\n";

    fn registry(name: &str, record: &str) -> (Registry, Store) {
        let path = dir(name).join("state.json");
        let reg = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        fs::create_dir_all(&reg.providers).unwrap();
        fs::write(reg.providers.join("key.toml"), record).unwrap();
        (reg, Store::beside(&path))
    }

    fn account(reg: &mut Registry, store: &Store, name: &str) -> Arc<Terminal> {
        let key = reg.path.with_file_name(format!("{name}.key"));
        fs::write(&key, "sk-test").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let catalog = crate::providers::Catalog::load(&reg.providers).0;
        let add = AccountAdd {
            provider: "key".into(),
            name: name.into(),
            folder: None,
            token_file: Some(key),
        };
        store.add(catalog.by_id("key").unwrap(), &add).unwrap();
        let lane = format!("lane-{name}");
        let term = sleeper(reg, &lane);
        let entry = reg.entries.get_mut(&lane).unwrap();
        (entry.info.acp, entry.info.account) = (Some("key".into()), Some(name.into()));
        term
    }

    fn used(reg: &mut Registry, name: &str, input: u64, output: u64, cost: Option<f64>) {
        let usage = AcpUsage {
            used: 0,
            size: 0,
            cost: cost.map(|c| json!({"amount": c, "currency": "USD"})),
            turn: None,
            total: Some(Tokens {
                input,
                output,
                ..Tokens::default()
            }),
        };
        let kind = EventKind::Usage(usage);
        let p = EmitParams {
            name: format!("lane-{name}"),
            kind,
            replay: false,
        };
        reg.emit(p).unwrap();
    }

    fn spend(reg: &Registry, store: &Store, name: &str) -> Value {
        let catalog = crate::providers::Catalog::load(&reg.providers).0;
        let account = store.account(catalog.by_id("key").unwrap(), name).unwrap();
        let shown = crate::accounts::shown(reg, "key", account);
        serde_json::to_value(shown.spend).unwrap()
    }

    #[tokio::test]
    async fn each_account_totals_its_lanes_usage_this_month_only_and_a_restart_keeps_them() {
        let (mut reg, store) = registry("spend-month", PRICED);
        let lanes: Vec<_> = (["a", "b", "c"].iter())
            .map(|name| account(&mut reg, &store, name))
            .collect();
        let catalog = crate::providers::Catalog::load(&reg.providers).0;
        let key = catalog.by_id("key").unwrap();
        let last = Tokens {
            input: 1_000_000,
            ..Tokens::default()
        };
        for name in ["a", "c"] {
            store.spent(key, name, "1999-12", last, None).unwrap();
        }
        used(&mut reg, "a", 100_000, 10_000, None);
        used(&mut reg, "a", 250_000, 30_000, None);
        used(&mut reg, "b", 1_000, 1_000, Some(0.5));
        used(&mut reg, "b", 3_000, 2_000, Some(0.75));
        let month = crate::accounts::month();
        let tokens = |input, output| json!({"input": input, "cache_read": 0, "cache_write": 0, "output": output});
        let a = json!({"month": month, "tokens": tokens(250_000, 30_000), "cost": {"amount": 0.74, "currency": "USD"}});
        let b = json!({"month": month, "tokens": tokens(3_000, 2_000), "cost": {"amount": 0.75, "currency": "USD"}});
        assert_eq!(spend(&reg, &store, "a"), a);
        assert_eq!(spend(&reg, &store, "b"), b);
        assert_eq!(spend(&reg, &store, "c"), Value::Null);
        let path = reg.path.clone();
        drop(reg);
        let reg = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        assert_eq!(spend(&reg, &store, "a"), a);
        assert_eq!(spend(&reg, &store, "b"), b);
        for t in lanes {
            t.kill(Duration::ZERO).unwrap();
        }
        fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn usage_with_no_cost_and_no_price_gives_tokens_and_an_unknown_cost() {
        let record = "id = \"key\"\nagent = \"key\"\ntoken_var = \"KEY_API_KEY\"\n";
        let (mut reg, store) = registry("spend-unknown", record);
        let lane = account(&mut reg, &store, "a");
        used(&mut reg, "a", 400, 20, None);
        let spent = spend(&reg, &store, "a");
        assert_eq!(spent["tokens"]["input"], 400);
        assert_eq!(spent["tokens"]["output"], 20);
        assert_eq!(spent["cost"], "unknown");
        lane.kill(Duration::ZERO).unwrap();
        fs::remove_dir_all(reg.path.parent().unwrap()).ok();
    }
}
