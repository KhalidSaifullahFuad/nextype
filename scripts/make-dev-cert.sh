#!/bin/zsh
# Creates a self-signed code-signing identity "NexType Dev" in the login keychain.
# Why: macOS ties Accessibility / Screen Recording grants to the code signature; ad-hoc
# signatures change on every build and silently lose the grant. One stable identity fixes that.
# Undo: Keychain Access -> login -> delete "NexType Dev" (certificate + key).
set -euo pipefail
NAME="NexType Dev"
KC="$HOME/Library/Keychains/login.keychain-db"
if security find-identity -v -p codesigning | grep -q "\"$NAME\""; then
  echo "\"$NAME\" already exists"; exit 0
fi
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/cfg" <<CFG
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CFG
# /usr/bin/openssl is LibreSSL; its PKCS#12 output is what `security import` accepts.
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$TMP/cfg" \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" 2>/dev/null
/usr/bin/openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -out "$TMP/id.p12" -passout pass:nextype -name "$NAME"
security import "$TMP/id.p12" -k "$KC" -P nextype -T /usr/bin/codesign
echo "macOS will now ask you to allow trusting this certificate for code signing."
security add-trusted-cert -r trustRoot -p codeSign -k "$KC" "$TMP/cert.pem"
security find-identity -v -p codesigning | grep "\"$NAME\"" && echo "OK: \"$NAME\" ready"
