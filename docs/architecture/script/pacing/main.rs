use std::future::poll_fn;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream as StdStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
use tokio::sync::Semaphore;

const WORKERS: usize = 4;
const SCRIPTS: usize = 8;
const READS: usize = 1500;

#[derive(Clone, Copy, PartialEq)]
enum Yield {
    Wake,
    YieldNow,
}

fn spin(slice: Duration) {
    let start = Instant::now();
    let mut n = 0u64;
    while start.elapsed() < slice {
        for _ in 0..200 {
            n = std::hint::black_box(n.wrapping_mul(31).wrapping_add(7));
        }
    }
}

async fn hand_back(how: Yield) {
    match how {
        Yield::YieldNow => tokio::task::yield_now().await,
        Yield::Wake => {
            let mut yielded = false;
            poll_fn(|cx| {
                if yielded {
                    return Poll::Ready(());
                }
                yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            })
            .await;
        }
    }
}

async fn script(
    slice: Duration,
    how: Yield,
    permits: Option<Arc<Semaphore>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Relaxed) {
        match &permits {
            Some(p) => {
                let _stepping = p.acquire().await;
                spin(slice);
            }
            None => spin(slice),
        }
        hand_back(how).await;
    }
}

async fn serve(stream: StdStream) {
    stream.set_nonblocking(true).ok();
    let Ok(stream) = tokio::net::UnixStream::from_std(stream) else {
        return;
    };
    let (read, mut write) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if write
            .write_all(format!("{line}\n").as_bytes())
            .await
            .is_err()
        {
            return;
        }
    }
}

fn client(stream: StdStream) -> Vec<Duration> {
    let mut took = Vec::with_capacity(READS);
    let Ok(mut write) = stream.try_clone() else {
        return took;
    };
    let mut read = BufReader::new(stream);
    let mut line = String::new();
    for _ in 0..READS {
        std::thread::sleep(Duration::from_millis(1));
        let start = Instant::now();
        if write.write_all(b"read\n").is_err() {
            break;
        }
        line.clear();
        if read.read_line(&mut line).is_err() {
            break;
        }
        took.push(start.elapsed());
    }
    took
}

fn measure(name: &str, scripts: usize, slice: Duration, how: Yield, permits: Option<usize>) {
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
    else {
        return;
    };
    let stop = Arc::new(AtomicBool::new(false));
    let permits = permits.map(|n| Arc::new(Semaphore::new(n)));
    let Ok((ours, theirs)) = StdStream::pair() else {
        return;
    };
    let _in = runtime.enter();
    runtime.spawn(serve(theirs));
    for _ in 0..scripts {
        runtime.spawn(script(slice, how, permits.clone(), stop.clone()));
    }
    let mut took = client(ours);
    stop.store(true, Relaxed);
    took.sort();
    let at = |q: f64| {
        let i = ((took.len() as f64 - 1.0) * q).round() as usize;
        took.get(i).map_or(0.0, |d| d.as_secs_f64() * 1e3)
    };
    println!(
        "{name:<58} p50 {:>7.3} ms   p99 {:>7.3} ms   max {:>7.3} ms",
        at(0.5),
        at(0.99),
        at(1.0)
    );
    runtime.shutdown_background();
}

fn main() {
    let short = Duration::from_micros(400);
    let long = Duration::from_millis(50);
    println!("{WORKERS} workers, {READS} reads by another client, 1 ms apart");
    measure("no script", 0, short, Yield::YieldNow, None);
    measure(
        "8 scripts, 0.4 ms slices, wake itself",
        SCRIPTS,
        short,
        Yield::Wake,
        None,
    );
    measure(
        "8 scripts, 0.4 ms slices, yield_now",
        SCRIPTS,
        short,
        Yield::YieldNow,
        None,
    );
    measure(
        "8 scripts, 0.4 ms slices, yield_now, 3 step at once",
        SCRIPTS,
        short,
        Yield::YieldNow,
        Some(3),
    );
    measure(
        "8 scripts, 50 ms holds, yield_now",
        SCRIPTS,
        long,
        Yield::YieldNow,
        None,
    );
    measure(
        "8 scripts, 50 ms holds, yield_now, 3 step at once",
        SCRIPTS,
        long,
        Yield::YieldNow,
        Some(3),
    );
}
