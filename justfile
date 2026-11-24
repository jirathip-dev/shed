# Every gate, the same command CI and `drover turn done` run: gatekeeper's core and rust packs and the gates in
# .gatekeeper/gates/.
check:
    ./.gatekeeper/run check

# The generated files in .drover/arch.toml (scripts/generate.sh).
generate:
    scripts/generate.sh

# The script engine's speed report (#214): targets 3, 4 and 5, each the median and range of 5 runs.
script-bench:
    cargo test -p shed-script --release --locked --test perf speed_report -- --ignored --nocapture --test-threads 1

# Build shed and install it to BIN_DIR/shed (default ~/.cargo/bin), signed on macOS with "herdsman-local" (#56).
install *BIN_DIR:
    scripts/install.sh {{BIN_DIR}}

# Every workspace test, crate by crate within its test_secs of CPU time (scripts/budget.py).
test:
    scripts/budget.py
