# CaptureOne for LG webOS TV

webOS TV packaging (`.ipk`) of the shared TV companion app in [`apps/tv-web`](../tv-web).

The app is a **companion**: it pairs with CaptureOne on a computer, browses and plays the
computer's recordings, and starts/pauses/stops recording there. It never records the TV
screen. LG's developer forum states that webOS TV provides no screen-capture API to apps.

## Build

```sh
pnpm install
pnpm --filter @captureone/tv-web build      # ES2015 classic-script bundle in apps/tv-web/dist
node apps/lg-tv/scripts/build.mjs           # -> artifacts/webos/dev.captureone.tv_<version>_all.ipk
```

The script validates `appinfo.json` and the icon sizes (80x80 `icon`, 130x130 `largeIcon`,
1920x1080 `splashBackground`), packages with `ares-package` from `@webos-tools/cli`, then opens
the `.ipk` (an `ar` archive with `debian-binary`, `control.tar.gz`, `data.tar.gz`) and checks
the control fields and that every app file is under `usr/palm/applications/dev.captureone.tv/`.
A listing is written next to the package (`*.ipk.contents.txt`).

`requiredPermissions` and `requiredACG` are empty: the only Luna call
(`luna://com.webos.service.tv.systemproperty/getSystemInfo`, for model/firmware) is optional.
If newer firmware denies it, the app falls back to `webOS.deviceInfo` / `PalmSystem.deviceInfo`.
`webOSTV.js` is not bundled (it is downloaded from LG's site); the app feature-detects it.

## Install on a TV (not verified here: no LG TV was available)

1. Create an LG developer account at webostv.developer.lge.com.
2. On the TV, install **Developer Mode** from the LG Content Store, sign in with that account and
   switch **Dev Mode Status** and **Key Server** on. Note the IP address and passphrase. The
   developer-mode session expires (extend it in the app), and apps installed this way are
   removed when it does.
3. On the computer (`npm i -g @webos-tools/cli`):
   ```sh
   ares-setup-device --add tv --info "{'host':'<TV IP>','port':'9922','username':'prisoner'}"
   ares-novacom --device tv --getkey          # enter the passphrase shown on the TV
   ares-install --device tv artifacts/webos/dev.captureone.tv_0.1.0_all.ipk
   ares-launch --device tv dev.captureone.tv
   ```
4. In CaptureOne on the computer enable the companion hub, then pair from the TV's
   **Connect** screen with the address and 6-digit code it shows.

Publishing to the LG Content Store requires LG's seller registration and review.

## Remote keys

Arrows move focus by geometry, OK activates, **Back** (keyCode 461; `disableBackHistoryAPI` is
set so the app receives it) goes back and exits from Home via `webOS.platformBack()` /
`PalmSystem.platformBack()`. Number keys type into the pairing fields; Play/Pause/Stop/FF/REW
control playback and remote recording.
