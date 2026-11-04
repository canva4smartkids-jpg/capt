import type { App, Screen } from "../app.js";
import { HubClient, HubError, normalizeHubUrl } from "../hub-client.js";
import { KeypadModel, type KeypadKey } from "../keypad.js";
import type { KeyAction } from "../keys.js";
import { loadLastAddress, saveLastAddress } from "../storage.js";
import { button, clear, formatDate, h } from "../ui/dom.js";

type Field = "address" | "code";

export function deviceName(app: App): string {
  const model = app.deviceInfo?.model;
  return model ? `${app.adapter.label} (${model})` : app.adapter.label;
}

export function createConnect(app: App): Screen {
  const address = new KeypadModel("address", loadLastAddress());
  const code = new KeypadModel("code");
  let active: Field = address.value ? "code" : "address";
  let busy = false;
  let fieldAddr: HTMLButtonElement;
  let fieldCode: HTMLButtonElement;
  let statusEl: HTMLElement;
  let keypadEl: HTMLElement;
  let pairedEl: HTMLElement;

  const setStatus = (msg: string, kind: "info" | "error" | "ok" = "info") => {
    statusEl.textContent = msg;
    statusEl.className = `status status-${kind}`;
  };

  const refreshFields = () => {
    fieldAddr.className = `field ${active === "address" ? "field-active" : ""}`;
    fieldCode.className = `field ${active === "code" ? "field-active" : ""}`;
    fieldAddr.querySelector(".field-value")!.textContent = address.value || "e.g. 192.168.1.20:47800";
    fieldCode.querySelector(".field-value")!.textContent = code.value ? code.value.replace(/(\d{3})(?=\d)/, "$1 ") : "6 digits";
    const sepKeys = keypadEl.querySelectorAll<HTMLButtonElement>("[data-key='.'],[data-key=':']");
    for (let i = 0; i < sepKeys.length; i++) sepKeys[i]!.disabled = active === "code";
  };

  const press = (k: KeypadKey) => {
    (active === "address" ? address : code).press(k);
    refreshFields();
  };

  const pair = async () => {
    if (busy) return;
    const url = normalizeHubUrl(address.value);
    if (!url) {
      active = "address";
      refreshFields();
      setStatus("Enter the address shown in CaptureOne on the computer (companion hub settings).", "error");
      return;
    }
    if (!code.isComplete()) {
      active = "code";
      refreshFields();
      setStatus("Enter the 6-digit pairing code shown on the computer.", "error");
      return;
    }
    busy = true;
    setStatus(`Contacting ${url}…`);
    try {
      const client = new HubClient(url);
      const info = await client.info();
      setStatus(`Found “${info.name}”. Pairing…`);
      const report = app.report ?? undefined;
      const resp = await client.pair(code.value, deviceName(app), app.adapter.deviceKind, report);
      saveLastAddress(address.value);
      app.setConnection({ hubUrl: url, token: resp.token, hubName: resp.hub.name, hubId: resp.hub.hubId, pairedAt: new Date().toISOString() }, resp.hub, client);
      code.press("clear");
      setStatus(`Paired with “${resp.hub.name}”.`, "ok");
      app.toast(`Paired with ${resp.hub.name}`);
      renderPaired();
    } catch (e) {
      if (e instanceof HubError) {
        if (e.kind === "wrong-code") {
          setStatus(`Wrong code${e.remainingAttempts !== undefined ? ` (${e.remainingAttempts} attempts left before a pause)` : ""}. Check the code on the computer.`, "error");
          code.press("clear");
          active = "code";
          refreshFields();
        } else {
          setStatus(e.message, "error");
        }
      } else {
        setStatus("Pairing failed.", "error");
      }
    } finally {
      busy = false;
    }
  };

  const renderPaired = () => {
    clear(pairedEl);
    const c = app.connection;
    if (!c) {
      pairedEl.appendChild(h("p", { class: "muted" }, "Not paired yet."));
      return;
    }
    pairedEl.appendChild(h("div", { class: "kv" }, h("span", { class: "k" }, "Computer"), h("span", { class: "v", id: "paired-name" }, app.hubInfo?.name || c.hubName)));
    pairedEl.appendChild(h("div", { class: "kv" }, h("span", { class: "k" }, "Address"), h("span", { class: "v" }, c.hubUrl)));
    if (c.pairedAt) pairedEl.appendChild(h("div", { class: "kv" }, h("span", { class: "k" }, "Paired"), h("span", { class: "v" }, formatDate(c.pairedAt))));
    pairedEl.appendChild(
      h(
        "div",
        { class: "actions" },
        button("Open recordings", () => app.go("recordings"), { id: "go-recordings" }),
        button("Forget this computer", () => {
          app.disconnect();
          renderPaired();
          setStatus("Forgotten. Pair again with a new code.");
        }),
      ),
    );
  };

  const key = (k: KeypadKey, label: string, cls = "") =>
    h("button", { type: "button", class: `key ${cls}`.trim(), "data-focusable": true, "data-key": k, onclick: () => press(k) }, label);

  return {
    id: "connect",
    render(root) {
      fieldAddr = h(
        "button",
        { type: "button", class: "field", id: "field-address", "data-focusable": true, onclick: () => ((active = "address"), refreshFields()) },
        h("span", { class: "field-label" }, "Computer address"),
        h("span", { class: "field-value" }),
      );
      fieldCode = h(
        "button",
        { type: "button", class: "field", id: "field-code", "data-focusable": true, onclick: () => ((active = "code"), refreshFields()) },
        h("span", { class: "field-label" }, "Pairing code"),
        h("span", { class: "field-value" }),
      );
      keypadEl = h(
        "div",
        { class: "keypad", id: "keypad" },
        h("div", { class: "keyrow" }, key("1", "1"), key("2", "2"), key("3", "3"), key("del", "⌫ Delete", "key-wide")),
        h("div", { class: "keyrow" }, key("4", "4"), key("5", "5"), key("6", "6"), key(".", ".", "key-wide")),
        h("div", { class: "keyrow" }, key("7", "7"), key("8", "8"), key("9", "9"), key(":", ": (port)", "key-wide")),
        h(
          "div",
          { class: "keyrow" },
          key("clear", "Clear"),
          key("0", "0"),
          h("button", { type: "button", class: "key key-ok", id: "pair-button", "data-focusable": true, onclick: () => void pair() }, "Pair"),
        ),
      );
      statusEl = h("div", { class: "status", id: "connect-status", role: "status" });
      pairedEl = h("div", { class: "kv-list", id: "paired" });
      root.appendChild(
        h(
          "div",
          { class: "row" },
          h(
            "section",
            { class: "card card-connect" },
            h("h2", null, "Pair with CaptureOne on your computer"),
            h(
              "ol",
              { class: "steps" },
              h("li", null, "On the computer, open CaptureOne's settings and enable the companion hub for TVs and phones."),
              h("li", null, "Enter the address and the 6-digit code it shows. Use the number keys on your remote or the keypad."),
            ),
            fieldAddr,
            fieldCode,
            statusEl,
          ),
          h("section", { class: "card card-keypad" }, keypadEl),
        ),
      );
      root.appendChild(h("section", { class: "card" }, h("h2", null, "Paired computer"), pairedEl));
      refreshFields();
      renderPaired();
      (active === "address" ? fieldAddr : fieldCode).setAttribute("data-autofocus", "");
      app.on("connection", renderPaired);
    },
    onAction(a: KeyAction) {
      if (a.type === "digit") {
        press(a.digit as KeypadKey);
        if (active === "code" && code.isComplete()) {
          const ok = document.getElementById("pair-button");
          if (ok) app.nav.focus(ok);
        }
        return true;
      }
      return false;
    },
  };
}
