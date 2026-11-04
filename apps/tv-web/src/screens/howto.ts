import type { App, Screen } from "../app.js";
import { button, h } from "../ui/dom.js";

function section(title: string, ...body: (HTMLElement | string)[]): HTMLElement {
  return h("section", { class: "card guide", "data-focusable": true, tabindex: 0 }, h("h2", null, title), ...body);
}

function first(el: HTMLElement): HTMLElement {
  el.setAttribute("data-autofocus", "");
  return el;
}

export function createHowto(app: App): Screen {
  const isLg = app.adapter.id === "webos";
  return {
    id: "howto",
    render(root) {
      root.appendChild(
        first(section(
          "Why this TV cannot record itself",
          h(
            "p",
            null,
            isLg
              ? "LG's developer forum states that webOS TV provides no screen-capture API to apps, and there is no API to capture the sound either. CaptureOne for TV therefore never claims to record the TV. It remote-controls CaptureOne on your computer and plays back what the computer recorded."
              : "Samsung's Smart TV FAQ states that screen capture is not supported in Samsung TV apps, and there is no API to capture the sound either. CaptureOne for TV therefore never claims to record the TV. It remote-controls CaptureOne on your computer and plays back what the computer recorded.",
          ),
        )),
      );
      root.appendChild(
        section(
          "What you can record: the HDMI source",
          h(
            "p",
            null,
            "TVs have HDMI inputs, not outputs. You record the device that feeds the TV — a game console, set-top box, streaming stick or laptop — by splitting its HDMI signal into a capture card.",
          ),
          h(
            "ul",
            null,
            h("li", null, "A USB HDMI capture card (UVC, works without drivers) — 1080p60 is enough for most uses; 4K pass-through if your TV is 4K."),
            h("li", null, "An HDMI splitter, unless the capture card has a pass-through (loop-out) port."),
            h("li", null, "Two HDMI cables and a computer running CaptureOne."),
          ),
        ),
      );
      root.appendChild(
        section(
          "Wiring",
          h("pre", { class: "diagram" }, "Console / set-top box ──HDMI──▶ capture card (pass-through) ──HDMI──▶ TV\n                                     │\n                                    USB\n                                     ▼\n                         Computer with CaptureOne"),
        ),
      );
      root.appendChild(
        section(
          "Recording",
          h(
            "ol",
            { class: "steps" },
            h("li", null, "Pair this TV with the computer (Connect)."),
            h("li", null, "Open “Record remotely” and press “Start (HDMI capture card)”, or pick the capture card as the source in CaptureOne on the computer."),
            h("li", null, "Stop when done. The recording appears under Recordings and plays right here on the TV."),
          ),
        ),
      );
      root.appendChild(
        section(
          "Protected content (HDCP)",
          h(
            "p",
            null,
            "Streaming apps (Netflix, Disney+, Prime Video…), Blu-ray players and many set-top boxes encrypt HDMI with HDCP. Compliant capture cards then show black or “no signal”. CaptureOne does not bypass HDCP or any other copy protection — that content cannot be recorded.",
          ),
        ),
      );
      root.appendChild(
        section(
          "The TV's own apps and menus",
          h(
            "p",
            null,
            isLg
              ? "The picture of the TV's built-in apps never leaves the TV over HDMI. Experimental, undocumented: on some LG webOS models CaptureOne on the computer can request a still screenshot of the TV picture over the network through an undocumented LG call (ssap://tv/executeOneShot) after you accept a pairing prompt on the TV. It is not an official LG API, depends on model and firmware, may stop working after updates, gives still images only, and protected content is black."
              : "The picture of the TV's built-in apps never leaves the TV over HDMI. Samsung's Smart TV FAQ states that screen capture is not supported in Samsung TV apps, and Samsung TVs offer no screenshot function to other devices, so the TV's own apps cannot be recorded.",
          ),
        ),
      );
      root.appendChild(h("div", { class: "actions" }, button("Pair with a computer", () => app.go("connect")), button("Back to Home", () => app.go("home"))));
    },
  };
}
