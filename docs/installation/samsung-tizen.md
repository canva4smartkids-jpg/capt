# Samsung Tizen TV installation (companion app)

Samsung TV apps cannot capture the TV screen (platform restriction). CaptureOne for Tizen is a **companion**: browse and play recordings from CaptureOne Desktop, start/stop desktop recordings, and view this TV's capabilities. To record what the TV shows, connect its HDMI output (through an HDMI splitter if needed) to a USB capture card on the desktop and pick the card as source.

Side-loading (developer):
1. On the TV: *Apps* → type `12345` on the remote → enable Developer mode and enter your PC's IP; restart the TV.
2. Sign `CaptureOne.wgt` with **your** Samsung certificate in Tizen Studio (Certificate Manager → Samsung → TV). The sprint build is signed with a generic author certificate or unsigned and will not install on retail TVs as-is.
3. `sdb connect <tv-ip>` then `tizen install -n CaptureOne.wgt -t <device>`.

Pairing: on the desktop enable *Devices → Companion hub*, then on the TV enter the desktop IP and the 6-digit code.
