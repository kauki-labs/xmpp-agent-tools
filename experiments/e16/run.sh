#!/usr/bin/env bash
# E16: run the tokio-xmpp spike against a throwaway Prosody (direct TLS, MUC with archive, file share).
# Needs: PROSODY_BIN (the prosody binary; prosodyctl must sit next to it), openssl and cargo on PATH.
# Everything lives in $E16_WORK (default ./work, ignored by git); the test CA never enters any trust store:
# the spike reads it through SSL_CERT_FILE.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${E16_WORK:-$here/work}
prosody=${PROSODY_BIN:?set PROSODY_BIN to the prosody binary}
prosodyctl=$(dirname "$prosody")/prosodyctl
pid=""
cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; }
trap cleanup EXIT

rm -rf "$work" && mkdir -p "$work/pki" "$work/data"
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$work/pki/ca.key" -out "$work/pki/ca.pem" -days 2 -subj "/CN=e16-test-ca" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -keyout "$work/pki/srv.key" -out "$work/pki/srv.csr" -subj "/CN=localhost" >/dev/null 2>&1
printf 'subjectAltName=DNS:localhost,DNS:rooms.localhost,DNS:upload.localhost,IP:127.0.0.1\n' >"$work/pki/ext.cnf"
openssl x509 -req -in "$work/pki/srv.csr" -CA "$work/pki/ca.pem" -CAkey "$work/pki/ca.key" -CAcreateserial \
  -out "$work/pki/srv.pem" -days 2 -extfile "$work/pki/ext.cnf" >/dev/null 2>&1
sed "s|@WORK@|$work|g" "$here/prosody.cfg.lua.in" >"$work/prosody.cfg.lua"

export PROSODY_CONFIG="$work/prosody.cfg.lua"
for user in alice bob; do "$prosodyctl" register "$user" localhost "pw-$user" >/dev/null; done
"$prosody" -F >"$work/prosody.out" 2>&1 &
pid=$!
for _ in $(seq 1 100); do (exec 3<>/dev/tcp/127.0.0.1/15223) 2>/dev/null && break; sleep 0.3; done

(cd "$here" && cargo build --quiet)
SSL_CERT_FILE="$work/pki/ca.pem" "$here/target/debug/e16"
