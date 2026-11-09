#!/usr/bin/env bash
set -euo pipefail
set +x
if [[ $# != 2 ]]; then
    echo 'Usage: sign-thor.sh aligned-unsigned.apk IW4LJ-Thor.apk' >&2
    exit 2
fi
signer="${ANDROID_HOME:?}/build-tools/35.0.0/apksigner"
"$signer" sign --ks "${IW4L_THOR_KEYSTORE:?}" \
    --ks-key-alias thor-test --ks-pass "file:${IW4L_THOR_PASSWORD_FILE:?}" \
    --alignment-preserved true --v1-signing-enabled false --v4-signing-enabled false \
    --out "$2" "$1"
cert=$("$signer" verify --print-certs "$2" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p')
test "$cert" = bce0bb760a1076dd7ba6d8502b78bb094b0fac6ab83e97d3448a7c7ec5242d88
sha256sum "$2"
