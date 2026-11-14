# Every CI gate, locally.
check: generate fmt clippy banned test doc deny cap deps hygiene neutral comments actionlint self-hosted arch lessons temp-left

# The generated files in .herdsman/arch.toml, written in the current folder (so a fixture gets its own):
# CHANGELOG.md's Unreleased section from changelog.d/, then docs/protocol.md and docs/shed.openrpc.json.
[no-cd]
generate:
    python3 -I "{{justfile_directory()}}/scripts/changelog.py"
    SHED_BLESS=1 cargo test -p shed-proto --features schema --locked --test protocol_doc

fmt:
    cargo fmt --all --check

clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    # At most 10 allows in the workspace, each with a reason (clippy checks the reason).
    test "$(grep -rE 'allow\(' crates | wc -l)" -le 10

# The banned APIs (#67): each fixture breaks one [workspace.lints] ban, and clippy must reject it.
banned:
    scripts/banned.sh

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
    scripts/proto-features.sh

hygiene:
    scripts/hygiene.sh

# No agent or provider name in core code (#64); the leaky fixture proves the check bites.
neutral:
    scripts/neutral-core.sh
    ! scripts/neutral-core.sh ci/fixtures/leaky-core

# The comment rule (#65); the two breaking fixtures prove the check bites, the allowed one that it passes.
comments:
    scripts/comment-rule.py
    ! scripts/comment-rule.py ci/fixtures/comment-plain
    ! scripts/comment-rule.py ci/fixtures/comment-private-doc
    scripts/comment-rule.py ci/fixtures/comment-allowed

# The GitHub workflows lint clean; the broken fixture proves the check bites.
actionlint:
    actionlint
    ! actionlint ci/fixtures/actionlint-bad.yml

# Self-hosted runners only for same-repo code (#93); the unguarded fixture proves the check bites.
self-hosted:
    scripts/self-hosted.py
    ! scripts/self-hosted.py ci/fixtures/self-hosted-unguarded

# Layers, banned calls (processes only in shed_core::runner, time and sleep only in shed_core::clock, the
# environment only in shed_server::config), size limits and generated files in .herdsman/arch.toml; one
# fixture breaks each. On a pull request CI sets ARCH_BASE=HEAD^1, the PR's base, for `--base`.
arch:
    drover arch-check ${ARCH_BASE:+--base "$ARCH_BASE"}
    cd ci/fixtures/arch-layer && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-spawn && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-process && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-sleep && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-env && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-system-time && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-instant && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-size && ! drover arch-check --config ../../../.herdsman/arch.toml
    cd ci/fixtures/arch-generated && ! drover arch-check --config ../../../.herdsman/arch.toml

# The lesson checks in .herdsman/rules.md: an active one that fails fails, a shadow one only reports.
lessons:
    drover lesson-check
    ! env -u GITHUB_STEP_SUMMARY drover lesson-check --rules ci/fixtures/lesson-active.md
    env -u GITHUB_STEP_SUMMARY drover lesson-check --rules ci/fixtures/lesson-shadow.md

# Tests leave no shed-* temp folders (run after `just test`); the fixture folder proves the check bites.
temp-left:
    scripts/temp-left.sh
    ! scripts/temp-left.sh ci/fixtures/temp-left

# The CLI timing report (soft target: a non-waiting call under 20 ms) and the history throughput report.
bench:
    cargo test -p shed --locked --test cli timing_report -- --nocapture
    cargo test -p shed-core --locked --test history throughput_report -- --nocapture

# Build shed and install it to BIN_DIR/shed (default ~/.cargo/bin), signed on macOS with "herdsman-local" (#56).
install *BIN_DIR:
    scripts/install.sh {{BIN_DIR}}
