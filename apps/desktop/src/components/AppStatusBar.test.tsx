import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store/app";
import { AppStatusBar } from "./AppStatusBar";
import { countryFlag } from "./country";

describe("AppStatusBar", () => {
  it("shows internet, public IP, location, and traffic totals", () => {
    useAppStore.setState({
      trafficTotals: { sent: 12_345_678, received: 1_099_511_627_776 },
      networkStatus: {
        state: "online",
        public_ip: "203.0.113.8",
        country_code: "IR",
        city: "Tehran",
        checked_at: "2026-08-13T00:00:00.000Z",
        detail: "Internet is reachable",
      },
    });

    render(<AppStatusBar />);

    expect(screen.getByRole("status")).toHaveTextContent("Internet connected");
    expect(screen.getByRole("status")).toHaveTextContent("203.0.113.8");
    expect(screen.getByRole("status")).toHaveTextContent("Tehran");
    expect(screen.getByRole("status")).toHaveTextContent("Iran");
    expect(screen.getByRole("status")).toHaveTextContent("Sent: 11.77 MiB");
    expect(screen.getByRole("status")).toHaveTextContent("Received: 1.00 TiB");
    expect(screen.getByRole("status").className).toMatch(/sticky/);
  });

  it("refreshes network status once when the IP section is clicked", async () => {
    const refreshNetworkStatus = vi.fn(async () => undefined);
    useAppStore.setState({
      networkRefreshing: false,
      refreshNetworkStatus,
      networkStatus: {
        state: "online",
        public_ip: "203.0.113.8",
        country_code: "IR",
        city: "Tehran",
        checked_at: "2026-08-13T00:00:00.000Z",
        detail: null,
      },
    });
    render(<AppStatusBar />);
    const [internet] = screen.getAllByRole("button", {
      name: "Refresh connection and IP status",
    });
    if (!internet) {
      throw new Error("internet refresh control is missing");
    }
    await userEvent.click(internet);
    expect(refreshNetworkStatus).toHaveBeenCalledOnce();
  });

  it("creates flags only for valid ISO country codes", () => {
    expect(countryFlag("US")).toBe("🇺🇸");
    expect(countryFlag("unknown")).toBe("");
    expect(countryFlag(null)).toBe("");
  });
});
