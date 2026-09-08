import { strict as assert } from "node:assert";
import { test, type TestContext } from "node:test";
import { isUpdating, wireUpdate } from "./update.ts";

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));
function fixture(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  let busy = false;
  let saves = true;
  let failInstall = false;
  let available = true;
  const calls: string[] = [];
  const status: string[] = [];
  let click = () => {};
  const button = {
    disabled: false,
    textContent: "Check for updates",
    addEventListener: (_: string, listener: () => void) => {
      click = listener;
    },
    removeEventListener: () => {
      click = () => {};
    },
  } as unknown as HTMLButtonElement;
  const notice = {
    textContent: "",
    classList: { remove: () => {} },
  } as unknown as HTMLElement;
  const windowBefore = Object.getOwnPropertyDescriptor(globalThis, "window");
  const documentBefore = Object.getOwnPropertyDescriptor(
    globalThis,
    "document",
  );
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      __TAURI_INTERNALS__: {
        invoke: async (command: string) => {
          calls.push(command);
          if (command === "check_for_update")
            return available
              ? { version: "9.0.0", current: "1.0.0", notes: "" }
              : null;
          if (failInstall) throw new Error("signature rejected");
          return undefined;
        },
      },
    },
  });
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: { activeElement: button },
  });
  let dispose = () => {};
  t.after(() => {
    dispose();
    if (windowBefore) Object.defineProperty(globalThis, "window", windowBefore);
    else Reflect.deleteProperty(globalThis, "window");
    if (documentBefore)
      Object.defineProperty(globalThis, "document", documentBefore);
    else Reflect.deleteProperty(globalThis, "document");
  });
  return {
    calls,
    status,
    button,
    notice,
    busy: (value: boolean) => {
      busy = value;
    },
    saves: (value: boolean) => {
      saves = value;
    },
    failInstall: (value: boolean) => {
      failInstall = value;
    },
    available: (value: boolean) => {
      available = value;
    },
    click: () => click(),
    start: () => {
      dispose = wireUpdate({
        button,
        notice,
        busy: () => busy,
        setStatus: (text) => {
          status.push(text);
        },
        focusStatus: () => {},
        flushSettings: async () => {
          calls.push("save");
          return saves;
        },
        setSettingsLocked: (locked) => {
          calls.push(locked ? "lock" : "unlock");
        },
        activityChanged: () => {},
      });
    },
  };
}

test("automatic updates wait for idle then save before installing exactly once", async (t) => {
  const f = fixture(t);
  f.busy(true);
  f.start();
  await settle();
  assert.deepEqual(f.calls, ["check_for_update"]);
  assert.match(f.notice.textContent!, /after the current work/);
  assert.deepEqual(
    f.status,
    [],
    "background checks must preserve review status",
  );
  f.click();
  t.mock.timers.tick(1000);
  await settle();
  assert.deepEqual(f.calls, ["check_for_update"]);
  f.busy(false);
  t.mock.timers.tick(1000);
  await settle();
  assert.deepEqual(f.calls, [
    "check_for_update",
    "lock",
    "save",
    "install_update",
    "unlock",
  ]);
  t.mock.timers.tick(1000);
  await settle();
  assert.equal(f.calls.filter((call) => call === "install_update").length, 1);
  assert.equal(isUpdating(), false);
});

test("failed settings persistence prevents update and allows a manual retry", async (t) => {
  const f = fixture(t);
  f.saves(false);
  f.start();
  await settle();
  assert.equal(f.calls.includes("install_update"), false);
  assert.match(f.notice.textContent!, /settings could not be saved/);
  assert.equal(f.button.disabled, false);
  f.saves(true);
  f.click();
  await settle();
  assert.equal(f.calls.filter((call) => call === "install_update").length, 1);
});

test("a failed signed install reports failure and releases the activity lock", async (t) => {
  const f = fixture(t);
  f.failInstall(true);
  f.start();
  await settle();
  assert.match(f.notice.textContent!, /Could not install.*signature rejected/);
  assert.equal(f.calls.at(-1), "unlock");
  assert.equal(isUpdating(), false);
  assert.equal(f.button.disabled, false);
});

test("resident update checks run every four hours without changing idle status", async (t) => {
  const f = fixture(t);
  f.available(false);
  f.start();
  await settle();
  assert.deepEqual(f.calls, ["check_for_update"]);
  assert.deepEqual(f.status, []);
  t.mock.timers.tick(4 * 60 * 60 * 1000 - 1);
  await settle();
  assert.equal(f.calls.length, 1);
  t.mock.timers.tick(1);
  await settle();
  assert.deepEqual(f.calls, ["check_for_update", "check_for_update"]);
  f.click();
  await settle();
  assert.equal(f.status.at(-1), "You are on the latest version");
});
