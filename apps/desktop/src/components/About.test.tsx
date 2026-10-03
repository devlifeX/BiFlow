import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { About } from "./About";
import { useAppStore } from "../store/app";
import i18n from "../i18n/config";

vi.mock("../version", () => ({
  APP_VERSION: "1.2.0",
}));

describe("About", () => {
  afterEach(async () => {
    cleanup();
    await i18n.changeLanguage("en");
  });

  beforeEach(() => {
    useAppStore.setState({
      update: {
        phase: "idle",
        percent: null,
        version: null,
        error: null,
      },
    });
  });

  it.each([
    ["en", "Developers", "Dariush Vesal · Omis Asgari · Reza Mahdavi"],
    ["fa", "توسعه‌دهندگان", "داریوش وصال · امید عسگری · رضا مهدوی"],
  ])(
    "shows ordered developers, repository, and version in %s",
    async (language, label, names) => {
      await i18n.changeLanguage(language);
      render(<About />);
      expect(screen.getByText(label)).toBeInTheDocument();
      expect(screen.getByText(names)).toBeInTheDocument();
      expect(screen.getByText("1.2.0")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /devlifeX\/BiFlow/i }),
      ).toBeInTheDocument();
    },
  );

  it("renders update progress and retry when the store reports failure", () => {
    useAppStore.setState({
      update: {
        phase: "failed",
        percent: null,
        version: "1.3.0",
        error: "Signature verification failed",
      },
    });
    render(<About />);
    expect(screen.getByRole("status")).toHaveTextContent(
      /could not check for updates/i,
    );
    expect(
      screen.getByText("Signature verification failed"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /retry update/i }),
    ).toBeInTheDocument();
  });

  it("tells the operator to reopen after a Debian package install", () => {
    useAppStore.setState({
      update: {
        phase: "installed",
        percent: 100,
        version: "1.3.0",
        error: null,
      },
    });
    render(<About />);
    expect(screen.getByRole("status")).toHaveTextContent(
      /quit and open the app again/i,
    );
  });

  it("offers install when an update is available", async () => {
    const installUpdate = vi.fn(async () => undefined);
    useAppStore.setState({
      update: {
        phase: "available",
        percent: null,
        version: "1.3.0",
        error: null,
      },
      installUpdate,
    });
    render(<About />);
    await userEvent.click(
      screen.getByRole("button", { name: /install update 1\.3\.0/i }),
    );
    expect(installUpdate).toHaveBeenCalledOnce();
  });
});
