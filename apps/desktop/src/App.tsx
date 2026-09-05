import * as Dialog from "@radix-ui/react-dialog";
import {
  Activity,
  BookOpen,
  Download,
  ExternalLink,
  Info,
  Languages,
  LayoutDashboard,
  Moon,
  SettingsIcon,
  Sun,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import logo from "./assets/logo.png";
import { desktop } from "./api/desktop";
import type { StackPhase } from "./api/models";
import {
  AppButton,
  BUTTON_ICON_PX,
  IconOnlyButton,
} from "./components/AppButton";
import { About } from "./components/About";
import { BasicDashboard } from "./components/BasicDashboard";
import { Dashboard } from "./components/Dashboard";
import { AppStatusBar } from "./components/AppStatusBar";
import { InputContextMenu } from "./components/InputContextMenu";
import { Diagnostics } from "./components/Diagnostics";
import { DirectRules } from "./components/DirectRules";
import { Settings } from "./components/Settings";
import { PageSkeleton } from "./components/PageSkeleton";
import { SettingsApplyBanner } from "./components/SettingsApplyBanner";
import { BottomNav } from "./components/BottomNav";
import { UiModeSwitch } from "./components/UiModeSwitch";
import { isMobileViewport, subscribeMobileViewport } from "./lib/viewport";
import { readUiMode, writeUiMode, type UiMode } from "./lib/uiMode";
import { useAppStore } from "./store/app";

/** Only a live stack lights the border: running is green, paused is amber, and
 * every stopped or transitional phase leaves the window unringed. */
function connectionGlow(
  phase: StackPhase | undefined,
): "active" | "paused" | null {
  if (phase === "running") return "active";
  if (phase === "paused") return "paused";
  return null;
}

export function App() {
  const { i18n, t } = useTranslation();
  const store = useAppStore();
  const [uiMode, setUiMode] = useState<UiMode>(() => readUiMode());
  const [mobile, setMobile] = useState(isMobileViewport);
  const [dark, setDark] = useState(
    () =>
      (localStorage.getItem("biflow-theme") ??
        localStorage.getItem("iran-split-theme")) === "dark",
  );

  useEffect(() => subscribeMobileViewport(setMobile), []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    localStorage.setItem("biflow-theme", dark ? "dark" : "light");
  }, [dark]);

  useEffect(() => {
    const rtl = i18n.language === "fa";
    document.documentElement.dir = rtl ? "rtl" : "ltr";
    document.documentElement.lang = i18n.language;
  }, [i18n.language]);

  useEffect(() => {
    let unsubscribe: () => void = () => undefined;
    void store.initialize().then((result) => {
      unsubscribe = result;
    });
    return () => unsubscribe();
    // The Zustand action is stable for the store lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let unsubscribe: () => void = () => undefined;
    void desktop
      .subscribeNavigation((page) => {
        useAppStore.getState().setPage(page);
      })
      .then((result) => {
        unsubscribe = result;
      });
    return () => unsubscribe();
  }, []);

  useEffect(() => {
    const interval = window.setInterval(() => {
      void useAppStore.getState().refreshNetworkStatus();
    }, 30_000);
    return () => window.clearInterval(interval);
  }, []);

  useEffect(() => {
    const interval = window.setInterval(() => {
      void useAppStore.getState().refreshTrafficTotals();
    }, 2_000);
    return () => window.clearInterval(interval);
  }, []);

  if (store.loading) {
    return (
      <main className="h-full overflow-hidden p-5" aria-busy="true">
        <PageSkeleton page={store.page} />
      </main>
    );
  }

  const advanced = uiMode === "advanced";
  const missing =
    store.snapshot?.last_error?.remediation === "install_dependency";
  const missingId =
    store.snapshot?.last_error?.code === "MIHOMO_NOT_FOUND"
      ? "mihomo"
      : "hiddify";

  const glow = connectionGlow(store.snapshot?.phase);

  return (
    <div
      data-connection-glow={glow ?? "none"}
      className={`app-shell grid h-full overflow-hidden ${
        advanced ? "app-shell-advanced md:grid-cols-[15rem_1fr]" : "grid-cols-1"
      } ${glow ? `connection-glow connection-glow-${glow}` : ""}`}
    >
      {advanced && !mobile ? (
        <aside
          data-testid="sidebar-nav"
          className="app-sidebar hidden h-full min-h-0 flex-col border-r border-ink/10 bg-surface/85 p-4 backdrop-blur md:flex"
        >
          <div className="flex items-center gap-3 px-2 py-2">
            <img
              src={logo}
              alt=""
              className="h-10 w-10 rounded-xl object-contain"
            />
            <div className="min-w-0">
              <p className="font-semibold">{t("appName")}</p>
              <p className="text-xs text-muted">{t("tagline")}</p>
            </div>
          </div>
          <nav
            aria-label="Primary navigation"
            className="mt-4 grid flex-1 content-start gap-1"
          >
            <NavButton
              page="dashboard"
              label={t("dashboard")}
              icon={<LayoutDashboard />}
            />
            <NavButton page="rules" label={t("rules")} icon={<BookOpen />} />
            <NavButton
              page="diagnostics"
              label={t("diagnostics")}
              icon={<Activity />}
            />
            <NavButton
              page="settings"
              label={t("settings")}
              icon={<SettingsIcon />}
            />
            <NavButton page="about" label={t("about")} icon={<Info />} />
          </nav>
          <div className="mt-auto flex gap-1 px-1 pt-4">
            <ThemeButton dark={dark} setDark={setDark} />
            <LanguageButton
              language={i18n.language}
              change={(lng) => void i18n.changeLanguage(lng)}
            />
          </div>
        </aside>
      ) : null}

      <div className="flex min-h-0 min-w-0 flex-col overflow-hidden">
        <main className="mx-auto flex min-h-0 w-full max-w-6xl flex-1 flex-col overflow-hidden p-3 sm:p-5">
          <div className="shrink-0 pb-4">
            <UiModeSwitch
              mode={uiMode}
              onChange={(mode) => {
                setUiMode(mode);
                if (mode === "basic") {
                  useAppStore.getState().setPage("dashboard");
                }
              }}
            />
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            <SettingsApplyBanner />
            {!advanced && store.page !== "about" && store.snapshot ? (
              <BasicDashboard snapshot={store.snapshot} />
            ) : null}
            {advanced && store.page === "dashboard" && store.snapshot ? (
              <Dashboard snapshot={store.snapshot} />
            ) : null}
            {advanced && store.page === "rules" && store.rules ? (
              <DirectRules rules={store.rules} />
            ) : null}
            {advanced && store.page === "diagnostics" ? (
              <Diagnostics report={store.diagnostics} />
            ) : null}
            {advanced && store.page === "settings" && store.settings ? (
              <Settings settings={store.settings} />
            ) : null}
            {store.page === "about" ? <About /> : null}
          </div>
        </main>
        {mobile ? (
          <BottomNav
            onNavigate={(page) => {
              if (
                uiMode === "basic" &&
                page !== "dashboard" &&
                page !== "about"
              ) {
                writeUiMode("advanced");
                setUiMode("advanced");
              }
              useAppStore.getState().setPage(page);
            }}
          />
        ) : null}
        <AppStatusBar />
        <InputContextMenu />
      </div>

      {advanced ? (
        <Dialog.Root
          open={store.error !== null && store.installGuide === null}
          onOpenChange={(open) => !open && store.clearError()}
        >
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 bg-black/45" />
            <Dialog.Content className="fixed left-1/2 top-1/2 w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-2xl bg-surface p-6 shadow-2xl">
              <Dialog.Title className="text-lg font-semibold">
                Action could not be completed
              </Dialog.Title>
              <Dialog.Description className="mt-2 text-sm text-muted">
                {store.error}
              </Dialog.Description>
              {store.snapshot?.last_error ? (
                <p className="mt-3 rounded-lg bg-canvas p-3 font-mono text-xs text-muted">
                  Correlation ID: {store.snapshot.last_error.correlation_id}
                </p>
              ) : null}
              {missing ? (
                <AppButton
                  icon={<Download size={BUTTON_ICON_PX} aria-hidden />}
                  className="mt-4 rounded-xl bg-brand px-4 py-2.5 font-semibold text-white"
                  onClick={() => void store.installDependency(missingId)}
                >
                  {t("install")} {missingId === "mihomo" ? "Mihomo" : "Hiddify"}
                </AppButton>
              ) : null}
              <Dialog.Close
                className="absolute right-4 top-4 rounded-lg p-1 text-muted"
                aria-label="Close"
              >
                <X size={19} aria-hidden />
              </Dialog.Close>
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      ) : null}

      {advanced ? (
        <Dialog.Root
          open={store.installGuide !== null}
          onOpenChange={(open) => !open && store.clearInstallGuide()}
        >
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 bg-black/45" />
            <Dialog.Content className="fixed left-1/2 top-1/2 max-h-[min(36rem,calc(100vh-2rem))] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-auto rounded-2xl bg-surface p-6 shadow-2xl">
              <Dialog.Title className="text-lg font-semibold">
                {t("installFailedTitle")}
              </Dialog.Title>
              <Dialog.Description className="mt-2 text-sm text-muted">
                {t("installFailedBody")}
              </Dialog.Description>
              {store.error ? (
                <p className="mt-3 text-sm text-danger">{store.error}</p>
              ) : null}
              {store.installGuide ? (
                <ol className="mt-4 list-decimal space-y-2 ps-5 text-sm">
                  {store.installGuide.steps.map((step) => (
                    <li key={step}>{step}</li>
                  ))}
                </ol>
              ) : null}
              <div className="mt-5 flex flex-wrap gap-2">
                {store.installGuide ? (
                  <AppButton
                    icon={<ExternalLink size={BUTTON_ICON_PX} aria-hidden />}
                    className="rounded-xl bg-brand px-4 py-2.5 font-semibold text-white"
                    onClick={() => {
                      const url = store.installGuide?.download_url;
                      if (url) void desktop.openUrl(url);
                    }}
                  >
                    {t("openDownload")}
                  </AppButton>
                ) : null}
                <Dialog.Close className="inline-flex items-center justify-center gap-2 rounded-xl border border-ink/15 px-4 py-2.5 font-semibold">
                  <X size={BUTTON_ICON_PX} aria-hidden />
                  {t("close")}
                </Dialog.Close>
              </div>
              <Dialog.Close
                className="absolute right-4 top-4 rounded-lg p-1 text-muted"
                aria-label="Close"
              >
                <X size={19} aria-hidden />
              </Dialog.Close>
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      ) : null}
    </div>
  );
}

function NavButton({
  page,
  label,
  icon,
}: {
  page: "dashboard" | "rules" | "diagnostics" | "settings" | "about";
  label: string;
  icon: React.ReactNode;
}) {
  const { page: current, setPage } = useAppStore();
  const active = current === page;
  return (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      onClick={() => setPage(page)}
      className={`flex min-w-0 items-center gap-2 rounded-xl px-3 py-3 text-sm font-medium transition ${
        active
          ? "bg-brand/10 text-brand"
          : "text-muted hover:bg-ink/5 hover:text-ink"
      }`}
    >
      <span aria-hidden>{icon}</span>
      <span className="truncate">{label}</span>
    </button>
  );
}

function ThemeButton({
  dark,
  setDark,
}: {
  dark: boolean;
  setDark: (dark: boolean) => void;
}) {
  const label = dark ? "Use light theme" : "Use dark theme";
  return (
    <IconOnlyButton
      label={label}
      onClick={() => setDark(!dark)}
      className="rounded-xl p-2 text-muted hover:bg-ink/5 hover:text-ink"
    >
      {dark ? <Sun size={19} aria-hidden /> : <Moon size={19} aria-hidden />}
    </IconOnlyButton>
  );
}

function LanguageButton({
  language,
  change,
}: {
  language: string;
  change: (language: string) => void;
}) {
  return (
    <IconOnlyButton
      label="Change language"
      onClick={() => {
        const next = language === "fa" ? "en" : "fa";
        localStorage.setItem("biflow-language", next);
        change(next);
      }}
      className="rounded-xl p-2 text-muted hover:bg-ink/5 hover:text-ink"
    >
      <Languages size={19} aria-hidden />
    </IconOnlyButton>
  );
}
