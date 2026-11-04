import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { Button, Dialog, KeyboardShortcut, ProgressBar, SegmentedControl, Tabs, TabPanel, Toggle, ToastProvider, useToast } from "../src/index.js";

describe("Toggle", () => {
  it("exposes switch semantics and toggles", async () => {
    const onChange = vi.fn();
    render(<Toggle checked={false} onChange={onChange} label="Microphone" />);
    const sw = screen.getByRole("switch", { name: "Microphone" });
    expect(sw).toHaveProperty("ariaChecked", "false");
    await userEvent.click(sw);
    expect(onChange).toHaveBeenCalledWith(true);
  });
  it("shows the disabled reason as its description", () => {
    render(<Toggle checked={false} onChange={() => {}} label="System audio" disabled disabledReason="Not supported on Linux X11" />);
    const sw = screen.getByRole("switch", { name: "System audio" }) as HTMLButtonElement;
    expect(sw.disabled).toBe(true);
    expect(sw.getAttribute("aria-describedby")).toBeTruthy();
    expect(screen.getByText("Not supported on Linux X11")).toBeTruthy();
  });
});

function TabsHarness() {
  const [tab, setTab] = useState<"a" | "b" | "c">("a");
  return (
    <>
      <Tabs idPrefix="t" label="Sources" value={tab} onChange={setTab} tabs={[{ id: "a", label: "Displays" }, { id: "b", label: "Windows", disabled: true }, { id: "c", label: "Region" }]} />
      <TabPanel idPrefix="t" id="a" active={tab === "a"}>
        panel-a
      </TabPanel>
      <TabPanel idPrefix="t" id="c" active={tab === "c"}>
        panel-c
      </TabPanel>
    </>
  );
}

describe("Tabs", () => {
  it("moves selection with arrow keys, skipping disabled tabs", async () => {
    render(<TabsHarness />);
    const first = screen.getByRole("tab", { name: "Displays" });
    first.focus();
    await userEvent.keyboard("{ArrowRight}");
    const region = screen.getByRole("tab", { name: "Region" });
    expect(region.getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(region);
    expect(screen.getByRole("tabpanel").textContent).toBe("panel-c");
    await userEvent.keyboard("{Home}");
    expect(first.getAttribute("aria-selected")).toBe("true");
  });
});

function DialogHarness() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Open</Button>
      <Dialog open={open} onClose={() => setOpen(false)} title="Delete recording?" footer={<><Button onClick={() => setOpen(false)}>Cancel</Button><Button variant="danger">Delete</Button></>} />
    </>
  );
}

describe("Dialog", () => {
  it("focuses inside, traps Tab, closes on Escape and restores focus", async () => {
    render(<DialogHarness />);
    const opener = screen.getByRole("button", { name: "Open" });
    await userEvent.click(opener);
    const dialog = screen.getByRole("dialog", { name: "Delete recording?" });
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(document.activeElement?.textContent).toBe("Cancel");
    await userEvent.tab();
    expect(document.activeElement?.textContent).toBe("Delete");
    await userEvent.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});

function ToastHarness() {
  const toast = useToast();
  return (
    <>
      <Button onClick={() => toast.push({ tone: "success", title: "Saved", timeout: 1000 })}>ok</Button>
      <Button onClick={() => toast.push({ tone: "danger", title: "Disk full" })}>bad</Button>
    </>
  );
}

describe("Toast", () => {
  it("auto-dismisses success toasts but keeps errors as alerts", () => {
    vi.useFakeTimers();
    render(
      <ToastProvider>
        <ToastHarness />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByText("ok"));
    fireEvent.click(screen.getByText("bad"));
    expect(screen.getByText("Saved")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("Disk full");
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(screen.queryByText("Saved")).toBeNull();
    expect(screen.getByText("Disk full")).toBeTruthy();
    vi.useRealTimers();
  });
});

describe("misc display", () => {
  it("ProgressBar exposes aria values and clamps", () => {
    render(<ProgressBar value={1.4} label="Storage" />);
    expect(screen.getByRole("progressbar", { name: "Storage" }).getAttribute("aria-valuenow")).toBe("100");
  });
  it("KeyboardShortcut has an accessible name", () => {
    render(<KeyboardShortcut accelerator="CommandOrControl+Shift+R" platform="other" />);
    expect(screen.getByRole("img", { name: "Control + Shift + R" })).toBeTruthy();
  });
  it("SegmentedControl selects with arrow keys", async () => {
    const onChange = vi.fn();
    render(<SegmentedControl label="FPS" value="30" onChange={onChange} options={[{ value: "24", label: "24" }, { value: "30", label: "30" }, { value: "60", label: "60" }]} />);
    screen.getByRole("radio", { name: "30" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onChange).toHaveBeenCalledWith("60");
  });
});
