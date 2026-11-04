#!/bin/bash
# Runs INSIDE the vitalets/tizen-webos-sdk Docker image (Tizen CLI 2.5.x):
#   tizen-sign.sh <staged widget dir>
# Creates a throwaway author certificate, a security profile using it plus Tizen's public default
# distributor certificate, and signs/packages the directory as CaptureOne.wgt.
#
# A package signed this way is structurally valid and installable on the Tizen emulator, but a
# retail Samsung TV only installs packages signed with a Samsung-issued author+distributor
# certificate (Tizen Studio Certificate Manager + Samsung account, distributor cert bound to the
# TV's DUID). Set TIZEN_AUTHOR_P12 / TIZEN_AUTHOR_PASSWORD to use your own author certificate.
set -euo pipefail
SRC=${1:?usage: tizen-sign.sh <dir>}
DATA="$HOME/tizen-studio-data"
if [ -n "${TIZEN_AUTHOR_P12:-}" ]; then
  P12="$TIZEN_AUTHOR_P12"
  PW="${TIZEN_AUTHOR_PASSWORD:?TIZEN_AUTHOR_PASSWORD required with TIZEN_AUTHOR_P12}"
else
  PW=$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')
  tizen certificate -a captureone -p "$PW" -c US -n "CaptureOne build" -o CaptureOne -f captureone-author >/dev/null
  P12="$DATA/keystore/author/captureone-author.p12"
fi
tizen security-profiles add -n captureone -a "$P12" -p "$PW" -A >/dev/null
PROF="$DATA/profile/profiles.xml"
# Containers have no keyring, so the CLI can neither write nor read its .pwd files. Inline the
# author password (the CLI then asks for it on the console) and the public password of Tizen's
# default distributor certificate.
PWD_FILE="${P12%.p12}.pwd"
sed -i "s#${PWD_FILE}#${PW}#; s#$DATA/tools/certificate-generator/certificates/distributor/tizen-distributor-signer.pwd#tizenpkcs12passfordsigner#" "$PROF"
tizen cli-config "profiles.path=$PROF" >/dev/null
rm -f "$SRC/CaptureOne.wgt"
# The CLI reads the password from a console (System.console()), so give it a pty with `script`:
# answer "Author password:" and then "Save author password (Y/N)?" with N.
( sleep 5; echo "$PW"; sleep 3; echo N
  for _ in $(seq 1 180); do [ -f "$SRC/CaptureOne.wgt" ] && break; sleep 1; done; sleep 1 ) \
  | script -qec "tizen package -t wgt -s captureone -- $SRC" /dev/null | grep -v -i password || true
if [ ! -f "$SRC/CaptureOne.wgt" ]; then
  grep -E "ERROR|Caused" "$DATA/cli/logs/cli.log" | tail -5 >&2 || true
  echo "tizen-sign: packaging failed" >&2
  exit 1
fi
# Keep the generated author certificate out of the package directory; never ship the password.
echo "tizen-sign: signed $SRC/CaptureOne.wgt"
