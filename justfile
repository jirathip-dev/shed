# Every CI gate, locally.
check: fmt clippy test doc deny cap deps hygiene

fmt:
    cargo fmt --all --check

clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    # At most 10 allows in the workspace, each with a reason (clippy checks the reason).
    test "$(grep -rE 'allow\(' crates | wc -l)" -le 10

test:
    cargo test --workspace --locked

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

deny:
    cargo deny --locked check

cap:
    scripts/line-cap.sh

deps:
    scripts/dep-direction.sh

hygiene:
    scripts/hygiene.sh

# The CLI timing report (soft target: a non-waiting call under 20 ms) and the history throughput report.
bench:
    cargo test -p shed --locked --test cli timing_report -- --nocapture
    cargo test -p shed-core --locked --test history throughput_report -- --nocapture

# Build shed and install it to BIN_DIR/shed (default ~/.cargo/bin), signed on macOS with "herdsman-local" (#56).
install *BIN_DIR:
    scripts/install.sh {{BIN_DIR}}
