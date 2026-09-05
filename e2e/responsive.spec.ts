import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";

const viewports = [
  { name: "mobile", width: 390, height: 844 },
  { name: "tablet", width: 768, height: 1024 },
  { name: "small-desktop", width: 1024, height: 768 },
] as const;

function resolveScreenshotDir(): string {
  const preferred = "/opt/cursor/artifacts/screenshots";
  try {
    mkdirSync(preferred, { recursive: true });
    return preferred;
  } catch {
    const fallback = join(process.cwd(), "test-results/responsive-screenshots");
    mkdirSync(fallback, { recursive: true });
    return fallback;
  }
}

const screenshotDir = resolveScreenshotDir();

async function openAdvanced(page: Page) {
  await page.goto("/");
  await page.waitForFunction(
    "typeof window.__BIFLOW_RESET_MOCK === 'function'",
  );
  await page.evaluate(() => {
    window.__BIFLOW_RESET_MOCK?.();
    localStorage.setItem("biflow-ui-mode-v1", "advanced");
  });
  await page.reload();
  await expect(page.getByRole("radio", { name: "Advanced" })).toBeVisible();
}

async function layoutMetrics(page: Page) {
  return page.evaluate(() => {
    const shell = document.querySelector(".app-shell");
    const main = document.querySelector("main");
    const nav = document.querySelector("[data-testid='bottom-nav']");
    const sidebar = document.querySelector("[data-testid='sidebar-nav']");
    const status = document.querySelector("footer[role='status']");
    const hamburger = [...document.querySelectorAll("button")].some((button) =>
      /hamburger|menu/i.test(
        button.getAttribute("aria-label") ?? button.textContent ?? "",
      ),
    );
    const overlap =
      nav && status
        ? nav.getBoundingClientRect().bottom >
          status.getBoundingClientRect().top + 0.5
        : false;
    return {
      hamburger,
      hasBottomNav: Boolean(nav),
      hasSidebar: Boolean(
        sidebar && getComputedStyle(sidebar).display !== "none",
      ),
      overflowX:
        document.documentElement.scrollWidth >
        document.documentElement.clientWidth,
      overflowY:
        document.documentElement.scrollHeight >
        document.documentElement.clientHeight,
      shellOverflow: shell ? getComputedStyle(shell).overflow : null,
      mainBottom: main?.getBoundingClientRect().bottom ?? 0,
      navTop: nav?.getBoundingClientRect().top ?? null,
      statusTop: status?.getBoundingClientRect().top ?? 0,
      overlap,
    };
  });
}

test.describe("responsive viewports", () => {
  for (const viewport of viewports) {
    test(`lays out ${viewport.name} ${viewport.width}x${viewport.height}`, async ({
      page,
    }) => {
      await page.setViewportSize({
        width: viewport.width,
        height: viewport.height,
      });
      await openAdvanced(page);
      const metrics = await layoutMetrics(page);
      expect(metrics.hamburger).toBe(false);
      expect(metrics.overflowX).toBe(false);
      expect(metrics.overlap).toBe(false);
      if (viewport.width < 768) {
        expect(metrics.hasBottomNav).toBe(true);
        expect(metrics.hasSidebar).toBe(false);
        expect(metrics.navTop).not.toBeNull();
        expect(metrics.navTop ?? 0).toBeLessThanOrEqual(metrics.statusTop);
      } else {
        expect(metrics.hasBottomNav).toBe(false);
        expect(metrics.hasSidebar).toBe(true);
      }

      mkdirSync(screenshotDir, { recursive: true });
      const file = join(
        screenshotDir,
        `${viewport.name}-${viewport.width}x${viewport.height}.png`,
      );
      await page.screenshot({ path: file, fullPage: false });
      expect(dirname(file)).toBe(screenshotDir);

      const pages = [
        "List Management",
        "Diagnostics",
        "Settings",
        "About",
        "Dashboard",
      ];
      for (const name of pages) {
        await page.getByRole("button", { name }).click();
        const next = await layoutMetrics(page);
        expect(next.overflowX, name).toBe(false);
        expect(next.overlap, name).toBe(false);
        expect(next.hamburger, name).toBe(false);
      }

      await page.getByRole("button", { name: "Dashboard" }).click();
      const connect = page.getByRole("button", {
        name: "Connect",
        exact: true,
      });
      await connect.click();
      const processing = page.locator("[data-connection-action='connect']");
      await expect(processing).toHaveAttribute("data-processing", "true");
      const labelBox = await processing
        .locator(".connection-action-label")
        .boundingBox();
      const buttonBox = await processing.boundingBox();
      expect(labelBox).not.toBeNull();
      expect(buttonBox).not.toBeNull();
      expect(labelBox?.width ?? 0).toBeLessThanOrEqual(
        (buttonBox?.width ?? 0) + 1,
      );
      const clipped = await processing.evaluate((button) => {
        const label = button.querySelector(".connection-action-label");
        if (!(label instanceof HTMLElement)) {
          return true;
        }
        return (
          label.scrollWidth > label.clientWidth + 1 ||
          label.scrollHeight > label.clientHeight + 1 ||
          button.scrollWidth > button.clientWidth + 1
        );
      });
      expect(clipped).toBe(false);
    });
  }

  test("shows bottom navigation in Basic on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/");
    await page.waitForFunction(
      "typeof window.__BIFLOW_RESET_MOCK === 'function'",
    );
    await page.evaluate(() => {
      window.__BIFLOW_RESET_MOCK?.();
      localStorage.setItem("biflow-ui-mode-v1", "basic");
    });
    await page.reload();
    await expect(page.getByRole("radio", { name: "Basic" })).toBeChecked();
    await expect(page.getByTestId("bottom-nav")).toBeVisible();
    await page.getByRole("button", { name: "List Management" }).click();
    await expect(
      page.getByRole("heading", { name: "List Management" }),
    ).toBeVisible();
    await expect(page.getByRole("radio", { name: "Advanced" })).toBeChecked();
  });

  test("scrolls the live routing SVG into view on mobile after connect", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await openAdvanced(page);
    await expect(page.getByTestId("provider-summary")).toBeVisible();
    await expect(page.getByTestId("connection-status-strip")).toBeVisible();
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Protected split routing is active" }),
    ).toBeVisible();
    const routing = page.getByTestId("live-routing");
    await routing.scrollIntoViewIfNeeded();
    await expect(routing).toBeVisible();
  });
});
