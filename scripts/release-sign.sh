#!/bin/sh
# release-sign FILE (#160): sign FILE with the Developer ID Application certificate (hardened runtime, secure
# timestamp, identifier shed), notarize it with the App Store Connect API key, and verify both. The release
# workflow's macOS job runs it with the five secrets in its environment (docs/releases.md); any failure exits non-zero
# and the job publishes nothing. It follows drover's scripts/release-sign.sh.
set -eu
f=${1:?usage: release-sign.sh FILE}
: "${MACOS_CERT_P12:?}" "${MACOS_CERT_PASSWORD:?}" "${ASC_API_KEY_P8:?}" "${ASC_API_KEY_ID:?}" "${ASC_API_ISSUER_ID:?}"
dir=$(mktemp -d)
kc="$dir/release.keychain-db"
trap 'security delete-keychain "$kc" 2>/dev/null; rm -rf "$dir"' EXIT
pw=$(openssl rand -hex 16)
security create-keychain -p "$pw" "$kc"
security set-keychain-settings -lut 3600 "$kc"
security unlock-keychain -p "$pw" "$kc"
printf %s "$MACOS_CERT_P12" | base64 --decode > "$dir/cert.p12"
security import "$dir/cert.p12" -k "$kc" -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$pw" "$kc" >/dev/null
# shellcheck disable=SC2046
security list-keychains -d user -s "$kc" $(security list-keychains -d user | tr -d '"')
printf %s "$ASC_API_KEY_P8" | base64 --decode > "$dir/key.p8"
codesign --force --options runtime --timestamp --identifier shed --keychain "$kc" \
  --sign "Developer ID Application" "$f"
ditto -c -k --keepParent "$f" "$dir/shed.zip"
out=$(xcrun notarytool submit "$dir/shed.zip" --key "$dir/key.p8" --key-id "$ASC_API_KEY_ID" \
  --issuer "$ASC_API_ISSUER_ID" --wait --output-format json)
echo "$out"
echo "$out" | grep -q '"status" *: *"Accepted"' || { echo "notarization not accepted" >&2; exit 1; }
codesign -dv --verbose=2 "$f" 2>&1 | tee "$dir/sig"
grep -q '^Authority=Developer ID Application' "$dir/sig" && grep -q '^Identifier=shed$' "$dir/sig"
codesign -v -R=notarized --check-notarization "$f"
spctl -a -vv -t install "$f"
