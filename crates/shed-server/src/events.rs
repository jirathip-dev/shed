use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use shed_proto::{Event, EventKind, EventsParams, TerminalInfo};
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::watch;

use crate::{HEARTBEAT, lock, send};

pub(crate) const RING: u64 = 4_096;
const TEXT_RING: usize = 256;
const BEAT: &[u8] = b"{\"jsonrpc\":\"2.0\",\"method\":\"heartbeat\"}\n";

type Labels = BTreeMap<String, String>;

pub(crate) type Line = Arc<[u8]>;

type Ring = VecDeque<(Event, Line)>;

#[derive(Default)]
struct Log {
    events: Ring,
    texts: BTreeMap<String, Ring>,
    seq: u64,
    evicted: u64,
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

fn notice(method: &str, seq: u64) -> Vec<u8> {
    format!("{{\"jsonrpc\":\"2.0\",\"method\":\"{method}\",\"params\":{{\"seq\":{seq}}}}}\n")
        .into_bytes()
}

impl Events {
    pub(crate) fn emit(&mut self, t: &TerminalInfo, kind: EventKind) {
        self.emit_as(t, kind, false);
    }

    pub(crate) fn emit_as(&mut self, t: &TerminalInfo, kind: EventKind, replay: bool) {
        self.push(Event {
            seq: 0,
            name: t.name.clone(),
            labels: t.labels.clone(),
            parent: t.parent.as_ref().map(|p| p.name.clone()),
            kind,
            replay,
        });
    }

    pub(crate) fn push(&mut self, mut event: Event) {
        self.seq += 1;
        event.seq = self.seq;
        let bytes = line(&event);
        let mut log = lock(&self.log);
        let log = &mut *log;
        log.seq = self.seq;
        if matches!(event.kind, EventKind::Text { .. }) {
            let ring = log.texts.entry(event.name.clone()).or_default();
            if ring.len() >= TEXT_RING {
                ring.pop_front();
            }
            ring.push_back((event, bytes));
        } else {
            if u64::try_from(log.events.len()).unwrap_or(u64::MAX) >= RING
                && let Some((gone, _)) = log.events.pop_front()
            {
                log.evicted = gone.seq;
                let evicted = log.evicted;
                (log.texts).retain(|_, ring| ring.back().is_some_and(|(e, _)| e.seq > evicted));
            }
            log.events.push_back((event, bytes));
        }
        self.last.send_replace(self.seq);
    }

    pub(crate) fn subscribe(&mut self, p: EventsParams) -> Subscriber {
        Subscriber {
            log: Arc::clone(&self.log),
            cursor: p.since.map_or(self.seq, |s| s.min(self.seq)),
            filter: p.labels,
            last: self.last.subscribe(),
        }
    }
}

pub(crate) struct Subscriber {
    log: Arc<Mutex<Log>>,
    cursor: u64,
    filter: Labels,
    last: watch::Receiver<u64>,
}

impl Subscriber {
    pub(crate) fn take(&mut self) -> Vec<Line> {
        self.take_with(|(_, line)| Arc::clone(line))
    }

    pub(crate) fn events(&mut self) -> Vec<Event> {
        self.take_with(|(event, _)| event.clone())
    }

    pub(crate) async fn changed(&mut self) -> bool {
        self.last.changed().await.is_ok()
    }

    fn missed(&self) -> Option<u64> {
        let evicted = lock(&self.log).evicted;
        (evicted > self.cursor).then_some(evicted)
    }

    fn take_with<T>(&mut self, f: impl FnMut(&(Event, Line)) -> T) -> Vec<T> {
        let log = lock(&self.log);
        let cursor = self.cursor;
        let rings = std::iter::once(&log.events).chain(log.texts.values());
        let mut kept: Vec<_> = rings
            .flat_map(|ring| ring.range(ring.partition_point(|(e, _)| e.seq <= cursor)..))
            .filter(|(e, _)| matches(&self.filter, e))
            .collect();
        kept.sort_unstable_by_key(|(e, _)| e.seq);
        self.cursor = self.cursor.max(log.seq);
        kept.into_iter().map(f).collect()
    }

    pub(crate) async fn stream(mut self, out: &mut OwnedWriteHalf, beats: bool) {
        if let Some(seq) = self.missed()
            && !send(out, &notice("reset", seq)).await
        {
            return;
        }
        loop {
            let sent = self.cursor;
            let batch = self.take().concat();
            if batch.is_empty() {
                let woke = if beats {
                    tokio::time::timeout(HEARTBEAT, self.last.changed()).await
                } else {
                    Ok(self.last.changed().await)
                };
                match woke {
                    Ok(Ok(())) => {}
                    Err(_) if send(out, BEAT).await => {}
                    _ => return,
                }
            } else {
                let (cursor, last) = (self.cursor, &mut self.last);
                let lagging = async {
                    let behind = |&seq: &u64| seq.saturating_sub(cursor) > RING;
                    last.wait_for(behind).await.is_ok()
                };
                tokio::select! {
                    written = send(out, &batch) => if !written { return },
                    lagged = lagging => {
                        if lagged {
                            out.try_write(&[&b"\n"[..], &notice("lagged", sent)].concat()).ok();
                        }
                        return;
                    }
                }
            }
            if self.missed().is_some() {
                send(out, &notice("lagged", self.cursor)).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _};

    pub(super) fn t(labels: &Labels) -> TerminalInfo {
        let t = serde_json::json!({"name": "t", "labels": labels, "cwd": "/", "cmd": [], "keep": false,
            "state": "running", "exit": null, "pid": null});
        serde_json::from_value(t).expect("a terminal")
    }

    fn named(name: &str) -> TerminalInfo {
        TerminalInfo {
            name: name.to_owned(),
            ..t(&Labels::new())
        }
    }

    fn text() -> EventKind {
        EventKind::Text {
            text: "chunk".to_owned(),
        }
    }

    fn seqs(sub: &mut Subscriber) -> Vec<u64> {
        sub.events().iter().map(|e| e.seq).collect()
    }

    #[test]
    fn the_log_keeps_the_last_ring_for_since() {
        let mut events = Events::default();
        let labels = Labels::from([("role".to_owned(), "impl".to_owned())]);
        for _ in 0..RING + 5 {
            events.emit(&t(&labels), EventKind::Started);
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
        assert_eq!(u64::try_from(seqs.len()), Ok(RING));
        assert_eq!((seqs[0], seqs[seqs.len() - 1]), (6, RING + 5));
        assert_eq!(all(&mut events, RING + 3), [RING + 4, RING + 5]);
    }

    #[test]
    fn every_subscriber_shares_the_one_serialised_line() {
        let mut events = Events::default();
        let mut subs: Vec<_> = (0..3)
            .map(|_| events.subscribe(EventsParams::default()))
            .collect();
        events.emit(&t(&Labels::new()), EventKind::Started);
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
            events.emit(&t(&Labels::new()), EventKind::Started);
        }
        assert_eq!(sub.take().len(), 3);
    }

    #[test]
    fn a_reconnect_after_text_floods_from_other_lanes_gets_every_state_event_in_order() {
        let mut events = Events::default();
        let mut state = Vec::new();
        for round in 0..5 {
            events.emit(&named("a"), EventKind::Killed);
            state.push(round * 1_001 + 1);
            for n in 0..1_000 {
                events.emit(&named(if n % 2 == 0 { "b" } else { "c" }), text());
            }
        }
        let mut sub = events.subscribe(EventsParams {
            labels: Labels::new(),
            since: Some(0),
        });
        let got = sub.events();
        let kept: Vec<u64> = (got.iter().filter(|e| e.name == "a"))
            .map(|e| e.seq)
            .collect();
        assert_eq!(kept, state);
        assert!(got.windows(2).all(|w| w[0].seq < w[1].seq), "in seq order");
        assert_eq!(
            got.len(),
            5 + 2 * TEXT_RING,
            "each lane keeps its last text chunks"
        );
    }

    #[test]
    fn text_rings_go_once_the_main_ring_has_moved_past_them() {
        let mut events = Events::default();
        events.emit(&named("old"), text());
        for _ in 0..=RING {
            events.emit(&named("a"), EventKind::Killed);
        }
        assert!(!lock(&events.log).texts.contains_key("old"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscriber_that_never_reads_is_dropped_and_the_log_stays_bounded() {
        let mut events = Events::default();
        let (server, mut client) = tokio::net::UnixStream::pair().expect("pair");
        let (_read, mut out) = server.into_split();
        let sub = events.subscribe(EventsParams::default());
        let stream = tokio::spawn(async move { sub.stream(&mut out, false).await });
        for n in 0..100_000 {
            events.emit(&t(&Labels::new()), EventKind::Started);
            if n % 64 == 0 {
                tokio::task::yield_now().await;
            }
        }
        assert!(stream.is_finished(), "dropped, not waiting on its write");
        assert!(u64::try_from(lock(&events.log).events.len()).is_ok_and(|n| n <= RING));
        let mut got = Vec::new();
        client
            .read_to_end(&mut got)
            .await
            .expect("read to the close");
        assert!(got.len() < 1 << 20, "{} bytes buffered", got.len());
    }

    #[tokio::test(start_paused = true)]
    async fn events_emitted_while_a_write_is_pending_are_sent_without_another_event() {
        let mut events = Events::default();
        let (server, client) = tokio::net::UnixStream::pair().expect("pair");
        let (_read, mut out) = server.into_split();
        let sub = events.subscribe(EventsParams::default());
        tokio::spawn(async move { sub.stream(&mut out, false).await });
        let text = "x".repeat(1 << 20);
        events.emit(&named("big"), EventKind::Text { text });
        tokio::task::yield_now().await;
        events.emit(&named("after"), EventKind::Killed);
        let mut lines = tokio::io::BufReader::new(client).lines();
        let mut names = Vec::new();
        let read = async {
            while let Ok(Some(line)) = lines.next_line().await {
                let event: Event = serde_json::from_str(&line).expect("an event");
                names.push(event.name);
                if names.len() == 2 {
                    return;
                }
            }
        };
        let read = tokio::time::timeout(HEARTBEAT, read).await;
        assert!(read.is_ok(), "only {names:?} arrived");
        assert_eq!(names, ["big", "after"]);
    }

    #[test]
    fn a_subscriber_more_than_the_ring_behind_has_missed_events() {
        let mut events = Events::default();
        let mut slow = events.subscribe(EventsParams::default());
        for _ in 0..=RING {
            events.emit(&t(&Labels::new()), EventKind::Started);
        }
        assert_eq!(slow.missed(), Some(1));
        assert_eq!(seqs(&mut slow).first(), Some(&2));
        assert_eq!(slow.missed(), None, "caught up");
    }

    async fn first_line(read: tokio::net::UnixStream) -> String {
        let mut lines = tokio::io::BufReader::new(read).lines();
        lines.next_line().await.expect("read").expect("a line")
    }

    #[tokio::test(start_paused = true)]
    async fn a_stale_since_starts_with_a_reset_and_a_reader_overrun_ends_with_lagged() {
        let mut events = Events::default();
        for _ in 0..=RING {
            events.emit(&t(&Labels::new()), EventKind::Started);
        }
        let (server, client) = tokio::net::UnixStream::pair().expect("pair");
        let (_read, mut out) = server.into_split();
        let since = Some(0);
        let sub = events.subscribe(EventsParams {
            since,
            ..EventsParams::default()
        });
        tokio::spawn(async move { sub.stream(&mut out, false).await });
        let reset = r#"{"jsonrpc":"2.0","method":"reset","params":{"seq":1}}"#;
        assert_eq!(first_line(client).await, reset);

        let (server, client) = tokio::net::UnixStream::pair().expect("pair");
        let (_read, mut out) = server.into_split();
        let sub = events.subscribe(EventsParams::default());
        let stream = tokio::spawn(async move { sub.stream(&mut out, false).await });
        tokio::task::yield_now().await;
        for _ in 0..=RING {
            events.emit(&t(&Labels::new()), EventKind::Started);
        }
        let mut lines = tokio::io::BufReader::new(client).lines();
        let mut last = String::new();
        while let Some(line) = lines.next_line().await.expect("read") {
            last = line;
        }
        assert!(stream.is_finished());
        assert!(last.contains(r#""method":"lagged""#), "{last}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_stream_beats_every_fifteen_seconds_only_when_asked() {
        let mut events = Events::default();
        for beats in [true, false] {
            let (server, client) = tokio::net::UnixStream::pair().expect("pair");
            let (_read, mut out) = server.into_split();
            let sub = events.subscribe(EventsParams::default());
            tokio::spawn(async move { sub.stream(&mut out, beats).await });
            let start = shed_core::clock::tokio_now();
            let line = tokio::time::timeout(HEARTBEAT * 2, first_line(client)).await;
            if beats {
                let line = line.ok();
                assert_eq!(line.as_deref().map(str::as_bytes), BEAT.strip_suffix(b"\n"));
                assert_eq!(start.elapsed(), HEARTBEAT);
            } else {
                assert!(line.is_err(), "silent without heartbeat");
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/bench/fanout.rs"]
mod bench;
