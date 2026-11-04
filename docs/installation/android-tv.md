# Android TV / Google TV installation

1. Enable *Developer options* (Settings → System → About → press *Android TV OS build* 7×) and *USB debugging* / *Install unknown apps*.
2. Install with `adb connect <tv-ip>` then `adb install captureone-tv-<version>.apk`, or with a side-loading app.
3. Launch **CaptureOne** from the apps row. The first screen reports what this TV supports:
   - **Native capture available** — the TV ships the system screen-capture consent dialog; recording works like on phones.
   - **Companion mode** — the TV lacks the consent dialog; pair with CaptureOne Desktop to browse/play recordings and remote-control recording (e.g. of this TV via an HDMI capture card).
