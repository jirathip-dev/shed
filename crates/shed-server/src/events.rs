use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use shed_proto::{Event, EventKind, EventsParams};
use tokio::io::AsyncWriteExt as _;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::watch;

use crate::lock;

pub(crate) const RING: u64 = 1_000;

type Labels = BTreeMap<String, String>;

pub(crate) type Line = Arc<[u8]>;

#[derive(Default)]
struct Log {
    events: VecDeque<(Event, Line)>,
    cursors: BTreeMap<u64, u64>,
}

#[derive(Default)]
pub(crate) struct Events {
    seq: u64,
    log: Arc<Mutex<Log>>,
    last: watch::Sender<u64>,
}

fn matches(filter: &Labels, event: &Event) -> bool {
    filter.iter().all(|(k, v)| event.labels.get(k) == Some(v))
}

fn line(event: &Event) -> Line {
    let mut line = serde_json::to_vec(event).unwrap_or_default();
    line.push(b'\n');
    line.into()
}

impl Events {
    pub(crate) fn emit(&mut self, name: &str, labels: &Labels, kind: EventKind) {
        self.emit_as(name, labels, kind, false);
    }

    pub(crate) fn emit_as(&mut self, name: &str, labels: &Labels, kind: EventKind, replay: bool) {
        self.seq += 1;
        let event = Event {
            seq: self.seq,
            name: name.to_owned(),
            labels: labels.clone(),
            kind,
            replay,
        };
        let bytes = line(&event);
        let mut log = lock(&self.log);
        log.events.push_back((event, bytes));
        if log.events.len() as u64 > 2 * RING {
            let slowest = log.cursors.values().min().copied();
            let floor = slowest.unwrap_or(u64::MAX).min(self.seq - RING);
            while log.events.front().is_some_and(|(e, _)| e.seq <= floor) {
                log.events.pop_front();
            }
        }
        self.last.send_replace(self.seq);
    }

    pub(crate) fn subscribe(&mut self, p: EventsParams) -> Subscriber {
        let mut log = lock(&self.log);
        let id = log.cursors.last_key_value().map_or(0, |(id, _)| id + 1);
        let oldest = self.seq.saturating_sub(RING);
        let cursor = p.since.map_or(self.seq, |s| s.clamp(oldest, self.seq));
        log.cursors.insert(id, cursor);
        Subscriber {
            log: Arc::clone(&self.log),
            id,
            filter: p.labels,
            last: self.last.subscribe(),
        }
    }
}

pub(crate) struct Subscriber {
    log: Arc<Mutex<Log>>,
    id: u64,
    filter: Labels,
    last: watch::Receiver<u64>,
}

impl Subscriber {
    pub(crate) fn take(&mut self) -> Vec<Line> {
        let mut log = lock(&self.log);
        let cursor = log.cursors.get(&self.id).copied().unwrap_or_default();
        let start = log.events.partition_point(|(e, _)| e.seq <= cursor);
        let lines = log.events.range(start..);
        let lines = lines.filter(|(e, _)| matches(&self.filter, e));
        let lines = lines.map(|(_, line)| Arc::clone(line)).collect();
        let end = log.events.back().map_or(cursor, |(e, _)| e.seq);
        log.cursors.insert(self.id, end);
        lines
    }

    pub(crate) async fn stream(mut self, out: &mut OwnedWriteHalf) {
        loop {
            let batch = self.take().concat();
            let written = batch.is_empty() || out.write_all(&batch).await.is_ok();
            if !written || self.last.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        lock(&self.log).cursors.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_last_thousand_for_since() {
        let mut events = Events::default();
        let labels = Labels::from([("role".to_owned(), "impl".to_owned())]);
        for _ in 0..1_005 {
            events.emit("t", &labels, EventKind::Started);
        }
        let all = |events: &mut Events, since| {
            let mut sub = events.subscribe(EventsParams {
                labels: labels.clone(),
                since: Some(since),
            });
            let text = String::from_utf8(sub.take().concat()).expect("utf-8");
            text.lines()
                .map(|l| serde_json::from_str::<Event>(l).expect("event").seq)
                .collect::<Vec<_>>()
        };
        let seqs = all(&mut events, 0);
        assert_eq!(seqs.len(), 1_000);
        assert_eq!((seqs[0], seqs[999]), (6, 1_005));
        assert_eq!(all(&mut events, 1_003), [1_004, 1_005]);
    }

    #[test]
    fn every_subscriber_shares_the_one_serialised_line() {
        let mut events = Events::default();
        let mut subs: Vec<_> = (0..3)
            .map(|_| events.subscribe(EventsParams::default()))
            .collect();
        events.emit("t", &Labels::new(), EventKind::Started);
        let lines: Vec<Line> = subs.iter_mut().flat_map(Subscriber::take).collect();
        let log = lock(&events.log);
        let kept = &log.events[0].1;
        assert!(lines.len() == 3 && lines.iter().all(|l| Arc::ptr_eq(l, kept)));
        assert_eq!(Arc::strong_count(kept), 4);
    }

    #[test]
    fn since_beyond_seq_still_gets_new_events() {
        let mut events = Events::default();
        let mut sub = events.subscribe(EventsParams {
            labels: Labels::new(),
            since: Some(5),
        });
        assert!(sub.take().is_empty());
        for _ in 0..3 {
            events.emit("t", &Labels::new(), EventKind::Started);
        }
        assert_eq!(sub.take().len(), 3);
    }

    #[test]
    fn a_slow_subscriber_misses_nothing_and_the_log_is_trimmed_without_one() {
        let mut events = Events::default();
        let mut slow = events.subscribe(EventsParams::default());
        for _ in 0..5_000 {
            events.emit("t", &Labels::new(), EventKind::Started);
        }
        assert_eq!(slow.take().len(), 5_000);
        assert!(slow.take().is_empty(), "the cursor moved past them");
        drop(slow);
        for _ in 0..5_000 {
            events.emit("t", &Labels::new(), EventKind::Started);
        }
        assert!(lock(&events.log).events.len() <= 2_000);
    }
}

#[cfg(test)]
#[path = "../tests/bench/fanout.rs"]
mod bench;
