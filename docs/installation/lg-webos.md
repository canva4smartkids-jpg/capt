# LG webOS TV installation (companion app)

webOS TV apps cannot capture the TV screen (platform restriction). CaptureOne for webOS is a **companion** (see the Tizen guide for the feature list and the HDMI capture-card workflow).

Side-loading (developer):
1. Install the *Developer Mode* app from the LG Content Store, sign in with an LG developer account, enable Dev Mode and Key Server, restart.
2. `ares-setup-device` (add the TV IP, passphrase from the Developer Mode app), then `ares-install --device <name> dev.captureone.tv_<version>_all.ipk`.

Pairing: same as Tizen. Separately, CaptureOne Desktop can discover LG TVs and send remote-control keys after you accept the pairing prompt on the TV; the one-shot TV screenshot feature is **experimental** (undocumented LG API) and may not work on your model.
