use std::time::Instant;

use super::tests::t;
use super::*;

const BURST: usize = 5_000;

#[test]
#[ignore = "a timing bench: scripts/bench-fanout.sh"]
fn fanout() {
    let labels = Labels::from(
        [("role", "impl"), ("fleet", "shed"), ("issue", "41")].map(|(k, v)| (k.into(), v.into())),
    );
    let mut total = 0;
    for n in [1, 16, 64, 256] {
        let us = (0..3)
            .map(|_| {
                let mut events = Events::default();
                let subs: Vec<_> = (0..n)
                    .map(|_| events.subscribe(EventsParams::default()))
                    .collect();
                let terminal = TerminalInfo {
                    name: "agent-terminal-1".into(),
                    ..t(&labels)
                };
                let start = Instant::now();
                for _ in 0..BURST {
                    events.emit(&terminal, EventKind::Started);
                }
                drop(events);
                let bytes: Vec<usize> = subs
                    .into_iter()
                    .map(|mut sub| sub.take().iter().map(|line| line.len()).sum())
                    .collect();
                let us = start.elapsed().as_micros();
                assert!(bytes.iter().all(|&b| b == bytes[0] && b > BURST));
                us
            })
            .min()
            .unwrap_or_default();
        println!("fanout n={n}: {us} us");
        total += us;
    }
    println!("fanout total: {total} us");
}
