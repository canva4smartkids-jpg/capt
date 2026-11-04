import type { SupportLevel } from "@captureone/capture-contracts";
import type { App, Screen } from "../app.js";
import { DISPLAYED_FEATURES, FEATURE_LABELS, LEVEL_EXPLANATIONS, LEVEL_LABELS } from "../capabilities.js";
import { button, clear, h } from "../ui/dom.js";

const LEVELS: SupportLevel[] = ["native", "partial", "companion-only", "external-hardware", "unsupported"];

export function badge(level: SupportLevel): HTMLElement {
  return h("span", { class: `badge badge-${level}`, "data-level": level }, LEVEL_LABELS[level] ?? level);
}

export function createHome(app: App): Screen {
  let infoBox: HTMLElement;
  let matrix: HTMLElement;

  const renderInfo = () => {
    clear(infoBox);
    const i = app.deviceInfo;
    const row = (k: string, v: string | undefined) => h("div", { class: "kv" }, h("span", { class: "k" }, k), h("span", { class: "v" }, v || "—"));
    infoBox.appendChild(row("Platform", app.adapter.label));
    if (!i) {
      infoBox.appendChild(h("p", { class: "muted" }, "Reading device information…"));
      return;
    }
    infoBox.appendChild(row("Model", i.model));
    infoBox.appendChild(row("Manufacturer", i.manufacturer));
    infoBox.appendChild(row("Operating system", `${i.osName} ${i.osVersion}`));
    infoBox.appendChild(row("Firmware", i.firmware));
    infoBox.appendChild(row("Resolution", i.resolution ? `${i.resolution.width} × ${i.resolution.height}` : undefined));
    infoBox.appendChild(row("Information from", i.sources.length ? i.sources.join(", ") : "none available"));
  };

  const renderMatrix = () => {
    clear(matrix);
    const r = app.report;
    if (!r) {
      matrix.appendChild(h("p", { class: "muted" }, "Building capability report…"));
      return;
    }
    for (const key of DISPLAYED_FEATURES) {
      const f = r.features[key];
      const level: SupportLevel = f ? f.level : "unsupported";
      matrix.appendChild(
        h(
          "div",
          { class: "cap-row", "data-focusable": true, tabindex: 0, "data-feature": key },
          h("div", { class: "cap-name" }, FEATURE_LABELS[key]),
          h("div", { class: "cap-level" }, badge(level)),
          h("div", { class: "cap-reason" }, f?.reason ?? ""),
        ),
      );
    }
  };

  return {
    id: "home",
    render(root) {
      infoBox = h("div", { class: "kv-list" });
      matrix = h("div", { class: "cap-matrix", id: "cap-matrix" });
      const paired = app.connection !== null;
      root.appendChild(
        h(
          "section",
          { class: "notice" },
          h("strong", null, "This app does not record the TV screen. "),
          "Samsung and LG TVs do not let apps capture the screen. CaptureOne for TV controls CaptureOne on your computer and plays its recordings. To record a picture shown on this TV, capture the HDMI signal with a capture card on the computer.",
        ),
      );
      root.appendChild(
        h(
          "div",
          { class: "row" },
          h(
            "section",
            { class: "card card-device" },
            h("h2", null, "This TV"),
            infoBox,
            h(
              "div",
              { class: "actions" },
              paired
                ? button("Open recordings", () => app.go("recordings"), { autofocus: true, id: "home-primary" })
                : button("Pair with a computer", () => app.go("connect"), { autofocus: true, id: "home-primary" }),
              button("How to record this TV", () => app.go("howto")),
            ),
          ),
          h(
            "section",
            { class: "card card-legend" },
            h("h2", null, "Support levels"),
            h(
              "ul",
              { class: "legend" },
              LEVELS.map((l) => h("li", null, badge(l), h("span", null, LEVEL_EXPLANATIONS[l]))),
            ),
          ),
        ),
      );
      root.appendChild(h("section", { class: "card" }, h("h2", null, "What this TV can do with CaptureOne"), matrix));
      renderInfo();
      renderMatrix();
      app.on("device", () => {
        renderInfo();
        renderMatrix();
      });
    },
  };
}
