#!/bin/sh
# Builds Dustoff.app into ~/Applications, so Accessibility is granted to Dustoff itself
# rather than to whichever terminal ran `cargo run`.
set -e
cd "$(dirname "$0")"
cargo build --release
app=~/Applications/Dustoff.app
rm -rf "$app" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp assets/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
cp target/release/dustoff "$app/Contents/MacOS/dustoff"
cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.cyrus.dustoff</string>
  <key>CFBundleName</key><string>Dustoff</string>
  <key>CFBundleExecutable</key><string>dustoff</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSUIElement</key><true/>
</dict></plist>
EOF
# A self-signed identity, made once in the login keychain, keeps the signature (and so the
# Accessibility grant) the same across rebuilds; ad-hoc signing changes it every build.
identity="Dustoff Local Signing"
if ! security find-certificate -c "$identity" >/dev/null 2>&1; then
  tmp=$(mktemp -d) && trap 'rm -rf "$tmp"' EXIT
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=$identity" \
    -addext "basicConstraints=critical,CA:false" -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=critical,codeSigning" -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
  # Legacy PKCS#12 ciphers: `security import` can't read OpenSSL 3's defaults.
  openssl pkcs12 -export -legacy -inkey "$tmp/key.pem" -in "$tmp/cert.pem" -out "$tmp/id.p12" -passout pass:dustoff
  security import "$tmp/id.p12" -k ~/Library/Keychains/login.keychain-db -P dustoff -T /usr/bin/codesign
fi
codesign --force --sign "$identity" "$app"
echo "Built $app"
