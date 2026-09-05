import { expect, test, type Page } from "@playwright/test";

async function openFresh(page: Page, mode: "basic" | "advanced" = "advanced") {
  await page.goto("/");
  await page.waitForFunction(
    "typeof window.__BIFLOW_RESET_MOCK === 'function'",
  );
  await page.evaluate(
    ([nextMode]) => {
      window.__BIFLOW_RESET_MOCK?.();
      localStorage.setItem("biflow-ui-mode-v1", nextMode);
    },
    [mode],
  );
  await page.reload();
  await expect(page.getByRole("radio", { name: "Advanced" })).toBeVisible();
}

function connectButton(page: Page) {
  return page.getByRole("button", { name: "Connect", exact: true });
}

async function expectNoDocumentOverflow(page: Page) {
  const overflow = await page.evaluate(() => ({
    horizontal:
      document.documentElement.scrollWidth >
      document.documentElement.clientWidth,
    vertical:
      document.documentElement.scrollHeight >
      document.documentElement.clientHeight,
  }));
  expect(overflow.horizontal).toBe(false);
  expect(overflow.vertical).toBe(false);
}

async function walkAdvancedPages(page: Page, labels: string[]) {
  for (const name of labels) {
    await page.getByRole("button", { name }).click();
    await expectNoDocumentOverflow(page);
  }
}

test.describe("primary BiFlow flows", () => {
  test("installs missing apps, connects, and splits traffic", async ({
    page,
  }) => {
    await openFresh(page);
    await expect(
      page.getByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    const statusBar = page.locator("footer[role='status']");
    await expect(statusBar).toContainText("Internet connected");
    await expect(statusBar).toContainText("198.51.100.24");
    await expect(statusBar).toContainText("🇮🇷");
    await expect(statusBar).toContainText("Sent: 1.00 MiB");
    await expect(statusBar).toContainText("Received: 2.00 MiB");
    await expect(page.getByText("unknown", { exact: true })).toHaveCount(0);

    const installButtons = page.getByRole("button", {
      name: "Install",
      exact: true,
    });
    await expect(installButtons).toHaveCount(2);
    await installButtons.nth(0).click();
    await expect(
      page.getByRole("button", { name: "Install", exact: true }),
    ).toHaveCount(1);
    await page.getByRole("button", { name: "Install", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Install", exact: true }),
    ).toHaveCount(0);

    await connectButton(page).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await expect(page.getByText("203.0.113.42")).toBeVisible();
    await expect(
      page.getByRole("img", {
        name: /traffic leaving this device and splitting/i,
      }),
    ).toBeVisible();
    await expect(page.locator(".traffic-flow-route")).toHaveCount(2);

    await page.getByRole("button", { name: "Pause" }).click();
    await expect(
      page.getByRole("heading", { name: "Split routing is paused" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
    await page.getByRole("button", { name: "Resume" }).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();

    await page.getByRole("button", { name: "Disconnect" }).click();
    await expect(
      page.getByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    await expect(page.locator(".traffic-flow-route")).toHaveCount(0);
  });

  test("disables lifecycle controls after the first Connect click", async ({
    page,
  }) => {
    await openFresh(page);
    const installButtons = page.getByRole("button", {
      name: "Install",
      exact: true,
    });
    await installButtons.nth(0).click();
    await page.getByRole("button", { name: "Install", exact: true }).click();
    const connect = page.locator("[data-connection-action='connect']");
    await expect(connect).toHaveAttribute("data-connect-glow", "available");
    await page.evaluate(() => {
      const seen: string[] = [];
      const record = () => {
        const label = document.querySelector(
          "[data-connection-action='connect'] .connection-action-label",
        );
        const text = label?.textContent?.trim();
        if (text && seen.at(-1) !== text) {
          seen.push(text);
        }
      };
      record();
      const observer = new MutationObserver(record);
      observer.observe(document.body, {
        subtree: true,
        childList: true,
        characterData: true,
      });
      window.__BIFLOW_STAGE_SEEN = seen;
      window.__BIFLOW_STAGE_STOP = () => observer.disconnect();
    });
    await connect.click();
    await expect(connect).toBeDisabled();
    await expect(connect).toHaveAttribute("data-processing", "true");
    await expect(connect).toHaveAttribute("data-connect-glow", "off");
    await connect.click({ force: true });
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Pause" })).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Disconnect" }),
    ).toBeEnabled();
    const stages = await page.evaluate(() => {
      window.__BIFLOW_STAGE_STOP?.();
      return window.__BIFLOW_STAGE_SEEN ?? [];
    });
    expect(stages).toEqual(
      expect.arrayContaining(["Start client", "Start Mihomo"]),
    );
  });

  test("connect installs missing apps before starting the stack", async ({
    page,
  }) => {
    await openFresh(page);
    await expect(
      page.getByRole("button", { name: "Install", exact: true }),
    ).toHaveCount(2);
    await connectButton(page).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Install", exact: true }),
    ).toHaveCount(0);
  });

  test("installs a missing helper from the advanced dashboard", async ({
    page,
  }) => {
    await openFresh(page);
    await page.evaluate(() => {
      sessionStorage.setItem("biflow-mock-force-missing-helper", "1");
      window.__BIFLOW_RESET_MOCK?.();
    });
    await page.reload();
    await expect(page.getByText("BiFlow")).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Setup needs attention" }),
    ).toBeVisible();
    await expect(
      page.getByText("Helper service is not installed or running"),
    ).toBeVisible();
    const installButtons = page.getByRole("button", {
      name: "Install",
      exact: true,
    });
    await expect(installButtons).toHaveCount(3);
    await installButtons.first().click();
    await expect(page.getByText("Mock helper is ready")).toBeVisible();
    await expect(installButtons).toHaveCount(2);
  });

  test("shows cloud rule counts and adds a custom direct rule", async ({
    page,
  }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "List Management" }).click();
    await expect(
      page.getByRole("heading", { name: "List Management" }),
    ).toBeVisible();
    await expect(
      page.getByText("62,828").or(page.getByText("62828")),
    ).toBeVisible();
    await expect(
      page.getByText("2,906").or(page.getByText("2906")),
    ).toBeVisible();

    await page.getByRole("button", { name: "Update from cloud" }).click();
    await expect(
      page.getByText("63,104").or(page.getByText("63104")),
    ).toBeVisible();

    await page.getByLabel("Domain or IP").fill("aparat.com");
    await page.getByRole("button", { name: "Add rule" }).click();
    await expect(
      page.getByRole("cell", { name: "aparat.com", exact: true }),
    ).toBeVisible();
  });

  test("diagnoses whether a host is direct or vpn", async ({ page }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "Diagnostics" }).click();
    await page.getByLabel("Test IP or domain").fill("openai.com");
    await page.getByRole("button", { name: "Test flow" }).click();
    await expect(page.getByText("openai.com → Hiddify")).toBeVisible();

    await page.getByLabel("Test IP or domain").fill("example.ir");
    await page.getByRole("button", { name: "Test flow" }).click();
    await expect(page.getByText("example.ir → DIRECT")).toBeVisible();

    // Reachability probes the fixed domains; disconnected mock keeps the VPN
    // pair red and iran.ir green, and a red row explains its likely causes.
    const reachability = page.getByTestId("reachability");
    await expect(
      reachability.getByRole("heading", { name: "Reachability", exact: true }),
    ).toBeVisible();
    await expect(reachability.getByText("google.com")).toBeVisible();
    await expect(reachability.getByText("facebook.com")).toBeVisible();
    await expect(reachability.getByText("iran.ir")).toBeVisible();
    // "Unreachable" contains "Reachable", so the green label needs exact.
    await expect(
      reachability.getByText("Reachable", { exact: true }),
    ).toBeVisible();
    await reachability.getByText("google.com").click();
    await expect(page.getByRole("dialog")).toContainText("Press Connect first");
    await page.getByRole("button", { name: "Close" }).click();

    await expect(
      page.getByRole("heading", { name: "Permanent debug.log", exact: true }),
    ).toBeVisible();
    await expect(page.getByTestId("debug-log-size")).toBeVisible();
    await page.getByRole("button", { name: "Show file" }).click();
    await expect(
      page.getByText("Opened the folder containing debug.log."),
    ).toBeVisible();
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Delete log" }).click();
    await expect(
      page.getByText(/previous log content was deleted/i),
    ).toBeVisible();
    await expect(page.getByTestId("debug-log-size")).toHaveText("512 B");
    await page.getByRole("button", { name: "Export" }).click();
    await expect(page.getByText(/Included:.*debug\.log/)).toBeVisible();
  });

  test("saves a DIRECT DNS preset from Settings", async ({ page }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("tab", { name: "Mihomo" }).click();
    const dns = page.getByLabel("DIRECT DNS");
    await expect(dns).toHaveValue("fake_ip");
    await dns.selectOption("mokhaberat");
    await page.getByRole("button", { name: "Save settings" }).click();
    await expect(
      page.getByRole("button", { name: "Save settings" }),
    ).toBeDisabled();
    await expect(dns).toHaveValue("mokhaberat");
  });

  test("rings the window green while connected and amber while paused", async ({
    page,
  }) => {
    await openFresh(page);
    const shell = page.locator("[data-connection-glow]");
    await expect(shell).toHaveAttribute("data-connection-glow", "none");

    const installButtons = page.getByRole("button", {
      name: "Install",
      exact: true,
    });
    await expect(installButtons).toHaveCount(2);
    await installButtons.nth(0).click();
    await expect(installButtons).toHaveCount(1);
    await installButtons.first().click();
    await expect(installButtons).toHaveCount(0);

    await connectButton(page).click();
    await expect(shell).toHaveAttribute("data-connection-glow", "active");
    await expect(shell).toHaveClass(/connection-glow-active/);

    await page.getByRole("button", { name: "Pause" }).click();
    await expect(shell).toHaveAttribute("data-connection-glow", "paused");
    await expect(shell).toHaveClass(/connection-glow-paused/);

    await page.getByRole("button", { name: "Resume" }).click();
    await expect(shell).toHaveAttribute("data-connection-glow", "active");

    await page.getByRole("button", { name: "Disconnect" }).click();
    await expect(shell).toHaveAttribute("data-connection-glow", "none");
    await expect(shell).not.toHaveClass(/connection-glow\b/);
  });

  test("pins an Iran host onto Hiddify and back to direct", async ({
    page,
  }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "Diagnostics" }).click();
    await page.getByLabel("Test IP or domain").fill("https://www.rade.ir/");
    await page.getByRole("button", { name: "Test flow" }).click();
    // The bundled Iran list keeps every .ir host direct until it is pinned.
    const flow = page.locator('[role="status"]').filter({
      hasText: "www.rade.ir",
    });
    await expect(flow.getByText("www.rade.ir → DIRECT")).toBeVisible();

    await flow.locator("select").selectOption({ label: "Hiddify" });
    await expect(flow.getByText("www.rade.ir → Hiddify")).toBeVisible();

    await page.getByRole("button", { name: "List Management" }).click();
    // Pins stay exactly as typed; the moved host keeps its www label.
    const pinned = page.getByRole("row").filter({
      has: page.getByText("www.rade.ir", { exact: true }),
    });
    await expect(pinned.locator("select")).toHaveValue(
      "11111111-1111-1111-1111-111111111111",
    );

    await pinned.locator("select").selectOption("direct");
    await expect(pinned.locator("select")).toHaveValue("direct");
    await page.getByRole("button", { name: "Diagnostics" }).click();
    await page.getByLabel("Test IP or domain").fill("www.rade.ir");
    await page.getByRole("button", { name: "Test flow" }).click();
    await expect(page.getByText("www.rade.ir → DIRECT")).toBeVisible();
  });

  test("restarts Hiddify on clean state from diagnostics", async ({ page }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "Diagnostics" }).click();
    await expect(
      page.getByRole("heading", { name: "Fresh Hiddify start", exact: true }),
    ).toBeVisible();

    page.once("dialog", (dialog) => dialog.dismiss());
    await page.getByRole("button", { name: "Fresh Hiddify start" }).click();
    await expect(
      page.getByText(/Hiddify restarted on clean runtime state/),
    ).toHaveCount(0);

    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Fresh Hiddify start" }).click();
    await expect(
      page.getByText(/Hiddify restarted on clean runtime state/),
    ).toBeVisible();
    await expect(page.getByText(/Kept: db\.sqlite/)).toBeVisible();
    await expect(page.getByText(/Backup: .*hiddify-/)).toBeVisible();
  });

  test("adds a catalog client, pins a host, and sets MATCH Direct", async ({
    page,
  }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "Add client" }).click();
    await expect(page.getByTestId("client-catalog")).toBeVisible();
    await page.getByRole("button", { name: /v2rayN/ }).click();
    await expect(page.getByTestId("client-card-v2rayn")).toBeVisible();
    const card = page.getByTestId("client-card-v2rayn");
    await card.getByPlaceholder("example.com").fill("openai.com");
    await card.getByRole("button", { name: "Pin" }).click();
    await expect(card.getByText("openai.com")).toBeVisible();
    await page.getByTestId("default-route").selectOption("direct");
    await card.getByRole("button", { name: "Delete" }).click();
    const confirm = card.getByRole("dialog");
    await expect(confirm).toContainText(
      "Delete v2rayN? This affects 1 pinned hosts.",
    );
    await expect(confirm.getByText("Move pins to")).toBeVisible();
    await confirm.locator("select").selectOption({ label: "Hiddify" });
    await confirm.getByRole("button", { name: "Delete" }).click();
    await expect(page.getByTestId("client-card-v2rayn")).toHaveCount(0);

    await connectButton(page).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
  });

  test("keeps the fixed viewport free of document overflow in English and Persian", async ({
    page,
  }) => {
    await openFresh(page);
    await walkAdvancedPages(page, [
      "Dashboard",
      "List Management",
      "Diagnostics",
      "Settings",
      "About",
    ]);

    await page.evaluate(() => {
      localStorage.setItem("biflow-language", "fa");
    });
    await page.reload();
    await expect(page.getByText("BiFlow")).toBeVisible();
    await walkAdvancedPages(page, [
      "داشبورد",
      "مدیریت لیست‌ها",
      "عیب‌یابی",
      "تنظیمات",
      "درباره",
    ]);
  });

  test("starts in Basic mode on first launch and can return to Advanced", async ({
    page,
  }) => {
    await openFresh(page, "basic");
    await expect(connectButton(page)).toBeVisible();
    await expect(
      page.getByRole("button", { name: "List Management" }),
    ).toHaveCount(0);
    await expectNoDocumentOverflow(page);

    await connectButton(page).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Disconnect" }).click();

    await page.getByRole("radio", { name: "Advanced" }).click();
    await expect(
      page.getByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "List Management" }),
    ).toBeVisible();
  });

  test("hides advanced chrome in Basic mode and can return to Advanced", async ({
    page,
  }) => {
    await openFresh(page);
    await page.getByRole("radio", { name: "Basic" }).click();
    await expect(connectButton(page)).toBeVisible();
    await expect(
      page.getByRole("button", { name: "List Management" }),
    ).toHaveCount(0);
    await expectNoDocumentOverflow(page);

    await connectButton(page).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Pause" }).click();
    await expect(
      page.getByRole("heading", { name: "Split routing is paused" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Resume" }).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Disconnect" }).click();
    await expect(connectButton(page)).toBeVisible();

    await page.getByRole("radio", { name: "Advanced" }).click();
    await expect(
      page.getByRole("heading", { name: "Ready when you are" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "List Management" }),
    ).toBeVisible();
  });

  test("offers select all, copy, cut, and paste on text inputs", async ({
    page,
  }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "Diagnostics" }).click();
    const field = page.getByLabel("Test IP or domain");
    await field.fill("example.ir");
    await field.evaluate((node) => {
      if (node instanceof HTMLInputElement) {
        node.focus();
        node.setSelectionRange(0, 0);
      }
    });
    await field.click({ button: "right" });
    const menu = page.getByTestId("input-context-menu");
    await expect(menu).toBeVisible();
    await expect(
      menu.getByRole("menuitem", { name: "Select All" }),
    ).toBeEnabled();
    await expect(menu.getByRole("menuitem", { name: "Copy" })).toBeDisabled();
    await expect(menu.getByRole("menuitem", { name: "Cut" })).toBeDisabled();
    await expect(menu.getByRole("menuitem", { name: "Paste" })).toBeEnabled();
    await menu.getByRole("menuitem", { name: "Select All" }).click();
    await expect(field).toHaveJSProperty("selectionStart", 0);
    await expect(field).toHaveJSProperty("selectionEnd", "example.ir".length);
    await field.click({ button: "right" });
    await expect(page.getByRole("menuitem", { name: "Copy" })).toBeEnabled();
    await expect(page.getByRole("menuitem", { name: "Cut" })).toBeEnabled();
  });

  test("pastes clipboard text into controlled diagnostics and direct-rule fields", async ({
    page,
    context,
  }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await openFresh(page);
    await page.evaluate(() =>
      navigator.clipboard.writeText("pasted.example.ir"),
    );

    await page.getByRole("button", { name: "Diagnostics" }).click();
    const diagnosticsField = page.getByLabel("Test IP or domain");
    await diagnosticsField.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Paste" }).click();
    await expect(diagnosticsField).toHaveValue("pasted.example.ir");

    await page.evaluate(() => navigator.clipboard.writeText("kavenegar.com"));
    await page.getByRole("button", { name: "List Management" }).click();
    const ruleField = page.getByLabel("Domain or IP");
    await ruleField.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Paste" }).click();
    await expect(ruleField).toHaveValue("kavenegar.com");
  });

  test("lists live DIRECT and VPN connections after connect", async ({
    page,
  }) => {
    await openFresh(page);
    await connectButton(page).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Diagnostics" }).click();
    const card = page.getByTestId("live-connections");
    await expect(card).toBeVisible();
    // The actions column is an outbound <select> that also lists DIRECT, so
    // assert the route badge span rather than every cell named DIRECT.
    const directRow = card.getByRole("row").filter({
      has: page.getByText("digikala.ir"),
    });
    await expect(
      directRow.locator("span", { hasText: "DIRECT" }),
    ).toBeVisible();
    const vpnRow = card.getByRole("row").filter({
      has: page.getByText("openai.com"),
    });
    await expect(vpnRow.locator("span", { hasText: "Hiddify" })).toBeVisible();
  });

  test("blocks the document context menu", async ({ page }) => {
    await openFresh(page);
    const prevented = await page.evaluate(() => {
      const event = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
      });
      document.dispatchEvent(event);
      return event.defaultPrevented;
    });
    expect(prevented).toBe(true);
  });

  test("shows About author, version, and update check", async ({ page }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "About" }).click();
    await expect(page.getByText("Dariush Vesal")).toBeVisible();
    await expect(page.getByText(/Version \d+\.\d+\.\d+/)).toBeVisible();
    await page.getByRole("button", { name: "Check for updates" }).click();
    await expect(page.getByText(/latest published version/i)).toBeVisible();
    await expectNoDocumentOverflow(page);

    await page.evaluate(() =>
      sessionStorage.setItem("biflow-mock-update-available", "1"),
    );
    await page.getByRole("button", { name: "Check for updates" }).click();
    await expect(page.getByText(/Version 9\.9\.9 is available/i)).toBeVisible();
    await page.getByRole("button", { name: /Install update 9\.9\.9/i }).click();
    await expect(page.getByRole("progressbar")).toBeVisible();
    await expect(
      page.getByText("an update is already in progress"),
    ).toHaveCount(0);
  });

  test("keeps a subdomain pin exact while connected", async ({ page }) => {
    await openFresh(page);
    await page.getByRole("button", { name: "List Management" }).click();
    await page.getByLabel("Domain or IP").fill("api.shop.example.com");
    await page.getByRole("button", { name: "Add rule" }).click();
    await expect(
      page.getByRole("cell", { name: "api.shop.example.com", exact: true }),
    ).toBeVisible();

    await page.getByRole("button", { name: "Diagnostics" }).click();
    await page.getByLabel("Test IP or domain").fill("www.technolife.com");
    await page.getByRole("button", { name: "Test flow" }).click();
    await expect(page.getByText("www.technolife.com → DIRECT")).toBeVisible();
  });
});
