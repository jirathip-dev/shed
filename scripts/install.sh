#!/bin/sh
# install [BIN_DIR] (#56): build shed and put it at BIN_DIR/shed (default ~/.cargo/bin), signed on macOS with the
# self-signed identity "herdsman-local" that herdsman and cooee also use (herdsman's scripts/signing-identity.sh
# creates it once). The designated requirement is then `identifier shed and certificate leaf = H"<sha-1>"` for every
# build, so a Keychain item that trusts one shed build opens to the next with no prompt.
# Signed by SHA-1 hash (the self-signed cert is not trusted, so `--sign herdsman-local` finds nothing). No identity,
# or a codesign that fails or waits on a dialog (killed after SHED_SIGN_TIMEOUT seconds, 10), keeps the old binary
# and exits 1. When the service job is already installed, `shed install` then restarts it on the new binary, so the
# running server is never an older build than the binary.
set -eu
dir=${1:-$HOME/.cargo/bin}
dest="$dir/shed"
cargo build --release --locked -p shed
mkdir -p "$dir"
cp "${CARGO_TARGET_DIR:-target}/release/shed" "$dest.new"
if command -v codesign >/dev/null; then
  limit() { perl -e 'alarm shift; exec @ARGV' "${SHED_SIGN_TIMEOUT:-10}" "$@"; }
  kc="$HOME/Library/Keychains/login.keychain-db"
  hash=$(limit security find-certificate -c herdsman-local -Z "$kc" 2>/dev/null | awk '/^SHA-1/ {print $NF}') || true
  if [ -z "$hash" ]; then
    rm -f "$dest.new"
    echo "shed install: no \"herdsman-local\" identity; kept $dest"
    echo "fix: sh scripts/signing-identity.sh in the herdsman checkout (once), then rerun this"
    exit 1
  fi
  if ! limit codesign --force --sign "$hash" --keychain "$kc" --identifier shed "$dest.new"; then
    rm -f "$dest.new"
    echo "shed install: codesign failed or timed out, likely waiting on a keychain dialog; kept $dest"
    echo "fix: security set-key-partition-list -S apple-tool:,apple:,codesign: -s $kc (asks the login password)"
    exit 1
  fi
fi
mv "$dest.new" "$dest"
echo "shed install: $dest"
for job in "$HOME/Library/LaunchAgents/dev.shed.serve.plist" "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/shed.service"; do
  if [ -e "$job" ]; then "$dest" install; fi
done
