# CaptureOne for Samsung Tizen TV

Tizen TV web-app packaging (`.wgt`) of the shared TV companion app in [`apps/tv-web`](../tv-web).

The app is a **companion**: it pairs with CaptureOne on a computer, browses and plays the
computer's recordings, and starts/pauses/stops recording there. It never records the TV
screen. Samsung's Smart TV FAQ states that screen capture is not supported in Samsung TV apps.

## Build

```sh
pnpm install
pnpm --filter @captureone/tv-web build          # apps/tv-web/dist
node apps/samsung-tv/scripts/build.mjs          # auto: signed if Docker works, else unsigned
node apps/samsung-tv/scripts/build.mjs --signer=docker   # require Tizen CLI signing
node apps/samsung-tv/scripts/build.mjs --signer=none     # unsigned zip only
```

Output: `artifacts/tizen/CaptureOne-<version>.wgt` (signed) or `CaptureOne-<version>-unsigned.wgt`,
plus a `*.contents.txt` listing.

The build validates `config.xml` (10-character package id `CaptureOne`, app id `CaptureOne.TV`,
`tv-samsung` profile, internet / `tv.inputdevice` / `productinfo` privileges, landscape, CSP that
allows LAN `http:`/`ws:`), stages `tv-web/dist`, injects `<script src="$WEBAPIS/webapis/webapis.js">`
(Samsung product-info API, resolved by the TV runtime) and signs with the Tizen CLI 2.5 from the
`vitalets/tizen-webos-sdk` Docker image ([`scripts/tizen-sign.sh`](scripts/tizen-sign.sh)). It then
re-computes every SHA-512 digest listed in `author-signature.xml` and `signature1.xml` and checks
that both signatures cover every file.

### What the signature is worth

The Docker build signs with a **throwaway author certificate** and **Tizen's public default
distributor certificate**. That produces a structurally valid, verifiable package (installable on
the Tizen TV emulator), but **retail Samsung TVs only install packages signed with a
Samsung-issued certificate**. To install on a real TV:

1. Install Tizen Studio with the TV extensions and **Samsung Certificate Extension**.
2. On the TV: open Apps, press `1 2 3 4 5` on the remote, switch **Developer mode** on, enter the
   computer's IP address, and restart the TV.
3. In Tizen Studio's Device Manager connect to the TV (or `sdb connect <TV IP>`), then in
   **Certificate Manager** create a *Samsung* certificate profile (signing in with a Samsung
   account; the distributor certificate is bound to the TV's DUID).
4. Re-package with that profile and install:
   ```sh
   tizen package -t wgt -s <samsung-profile> -- apps/samsung-tv/build/wgt-src
   tizen install -n CaptureOne.wgt -t <device name>
   tizen run -p CaptureOne.TV -t <device name>
   ```

Set `TIZEN_AUTHOR_P12` / `TIZEN_AUTHOR_PASSWORD` (inside the container) to sign with your own
author certificate. Certificates and passwords are never committed (`*.p12` is gitignored).

Not verified here: no Samsung TV, Tizen emulator or Samsung account was available.

## Remote keys

Arrows/OK/Return(10009) work without registration; the app registers `MediaPlayPause`, `MediaPlay`,
`MediaPause`, `MediaStop`, `MediaFastForward`, `MediaRewind`, track keys, digits and `Exit` with
`tizen.tvinputdevice.registerKey` and reads their codes with `getKey()`. Back on Home and Exit call
`tizen.application.getCurrentApplication().exit()`.
