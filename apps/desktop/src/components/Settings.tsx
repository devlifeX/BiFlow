import { zodResolver } from "@hookform/resolvers/zod";
import { Save } from "lucide-react";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import type { UseFormRegisterReturn } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { z } from "zod";
import { desktop } from "../api/desktop";
import type {
  AppConfig,
  DirectDnsPreset,
  ValidationIssue,
} from "../api/models";
import {
  DIRECT_DNS_PRESETS,
  DIRECT_DNS_PRESET_SERVERS,
  formatDirectDnsServers,
  parseDirectDnsServers,
} from "../lib/directDns";
import { useAppStore } from "../store/app";

const formSchema = z
  .object({
    controllerPort: z.coerce.number().int().min(1).max(65535),
    mixedPort: z.coerce.number().int().min(1).max(65535),
    dnsPort: z.coerce.number().int().min(1).max(65535),
    directDnsPreset: z.enum([
      "fake_ip",
      "shecan",
      "electro",
      "radar",
      "mokhaberat",
      "custom",
    ]),
    directDnsServers: z.string(),
    tunName: z
      .string()
      .min(1)
      .max(64)
      .regex(/^[a-zA-Z0-9_-]+$/),
    logLevel: z.enum(["error", "warn", "info", "debug"]),
    refreshMinutes: z.coerce.number().int().min(1),
    upstreamHours: z.coerce.number().int().min(1),
    launchAtLogin: z.boolean(),
    connectAtLaunch: z.boolean(),
    closeToTray: z.boolean(),
    failClosed: z.boolean(),
  })
  .superRefine((values, context) => {
    if (values.directDnsPreset !== "custom") return;
    if (parseDirectDnsServers(values.directDnsServers).length === 0) {
      context.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["directDnsServers"],
        message: "Enter at least one DNS IP address",
      });
    }
  });

type FormValues = z.infer<typeof formSchema>;
type SettingsTab = "mihomo" | "behavior";

function toValues(config: AppConfig): FormValues {
  return {
    controllerPort: config.mihomo.controller_port,
    mixedPort: config.mihomo.mixed_port,
    dnsPort: config.mihomo.dns_port,
    directDnsPreset: config.mihomo.direct_dns_preset,
    directDnsServers: formatDirectDnsServers(config.mihomo.direct_dns_servers),
    tunName: config.mihomo.tun_name,
    logLevel: config.mihomo.log_level,
    refreshMinutes: config.rules.refresh_interval_minutes,
    upstreamHours: config.rules.upstream_refresh_hours,
    launchAtLogin: config.behavior.launch_at_login,
    connectAtLaunch: config.behavior.connect_at_launch,
    closeToTray: config.behavior.close_to_tray,
    failClosed: config.behavior.fail_closed,
  };
}

function merge(config: AppConfig, values: FormValues): AppConfig {
  return {
    ...config,
    mihomo: {
      ...config.mihomo,
      controller_port: values.controllerPort,
      mixed_port: values.mixedPort,
      dns_port: values.dnsPort,
      tun_name: values.tunName,
      log_level: values.logLevel,
      direct_dns_preset: values.directDnsPreset,
      direct_dns_servers: parseDirectDnsServers(values.directDnsServers),
    },
    rules: {
      refresh_interval_minutes: values.refreshMinutes,
      upstream_refresh_hours: values.upstreamHours,
    },
    behavior: {
      launch_at_login: values.launchAtLogin,
      connect_at_launch: values.connectAtLaunch,
      close_to_tray: values.closeToTray,
      fail_closed: values.failClosed,
    },
  };
}

export function Settings({ settings }: { settings: AppConfig }) {
  const { t } = useTranslation();
  const { saveSettings, actionPending } = useAppStore();
  const [tab, setTab] = useState<SettingsTab>("mihomo");
  const [issues, setIssues] = useState<ValidationIssue[]>([]);
  const {
    register,
    handleSubmit,
    reset,
    watch,
    formState: { errors, isDirty },
  } = useForm<FormValues>({
    resolver: zodResolver(formSchema),
    defaultValues: toValues(settings),
  });
  const directDnsPreset = watch("directDnsPreset");

  useEffect(() => reset(toValues(settings)), [reset, settings]);

  const submit = handleSubmit(async (values) => {
    const draft = merge(settings, values);
    const validation = await desktop.validateSettings(draft);
    setIssues(validation);
    if (validation.length === 0) await saveSettings(draft);
  });

  return (
    <section aria-labelledby="settings-title" className="flex flex-col pb-2">
      <header className="shrink-0">
        <h1
          id="settings-title"
          className="text-2xl font-semibold tracking-tight"
        >
          Settings
        </h1>
        <p className="mt-1 text-sm text-muted">
          Advanced ports stay on loopback and are checked for conflicts before
          publication.
        </p>
      </header>

      <div
        role="tablist"
        aria-label="Settings sections"
        className="mt-4 flex shrink-0 gap-1 rounded-xl border border-ink/10 bg-canvas p-1"
      >
        <TabButton
          id="settings-tab-mihomo"
          selected={tab === "mihomo"}
          controls="settings-panel-mihomo"
          onSelect={() => setTab("mihomo")}
        >
          Mihomo
        </TabButton>
        <TabButton
          id="settings-tab-behavior"
          selected={tab === "behavior"}
          controls="settings-panel-behavior"
          onSelect={() => setTab("behavior")}
        >
          Behavior
        </TabButton>
      </div>

      <form
        onSubmit={(event) => void submit(event)}
        className="mt-4 flex flex-col"
      >
        <div className="min-h-0 flex-1 overflow-y-auto pe-1">
          {tab === "mihomo" ? (
            <Fieldset
              id="settings-panel-mihomo"
              labelledBy="settings-tab-mihomo"
              legend="Mihomo and network"
            >
              <Field
                label="Controller port"
                error={errors.controllerPort?.message}
              >
                <input
                  type="number"
                  {...register("controllerPort")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                />
              </Field>
              <Field label="Mixed port" error={errors.mixedPort?.message}>
                <input
                  type="number"
                  {...register("mixedPort")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                />
              </Field>
              <Field label="DNS port" error={errors.dnsPort?.message}>
                <input
                  type="number"
                  {...register("dnsPort")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                />
              </Field>
              <Field
                className="sm:col-span-2"
                label={t("settingsDns.label")}
                hint={t("settingsDns.help")}
                error={errors.directDnsPreset?.message}
              >
                <select
                  {...register("directDnsPreset")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                >
                  {DIRECT_DNS_PRESETS.map((preset) => (
                    <option key={preset} value={preset}>
                      {directDnsOptionLabel(preset, t(`settingsDns.${preset}`))}
                    </option>
                  ))}
                </select>
              </Field>
              {directDnsPreset === "custom" ? (
                <Field
                  className="sm:col-span-2"
                  label={t("settingsDns.customLabel")}
                  hint={t("settingsDns.customHelp")}
                  error={errors.directDnsServers?.message}
                >
                  <input
                    {...register("directDnsServers")}
                    placeholder={t("settingsDns.customPlaceholder")}
                    className="w-full rounded-xl border-ink/15 bg-canvas"
                  />
                </Field>
              ) : null}
              <Field label="TUN name" error={errors.tunName?.message}>
                <input
                  {...register("tunName")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                />
              </Field>
              <Field label="Log level" error={errors.logLevel?.message}>
                <select
                  {...register("logLevel")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                >
                  <option value="error">Error</option>
                  <option value="warn">Warning</option>
                  <option value="info">Info</option>
                  <option value="debug">Debug</option>
                </select>
              </Field>
            </Fieldset>
          ) : null}

          {tab === "behavior" ? (
            <Fieldset
              id="settings-panel-behavior"
              labelledBy="settings-tab-behavior"
              legend="Behavior and refresh"
            >
              <Field
                label="Custom rule refresh (minutes)"
                error={errors.refreshMinutes?.message}
              >
                <input
                  type="number"
                  {...register("refreshMinutes")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                />
              </Field>
              <Field
                label="Upstream refresh (hours)"
                error={errors.upstreamHours?.message}
              >
                <input
                  type="number"
                  {...register("upstreamHours")}
                  className="w-full rounded-xl border-ink/15 bg-canvas"
                />
              </Field>
              <div className="space-y-3 sm:col-span-2">
                <Check
                  label="Launch at login"
                  registration={register("launchAtLogin")}
                />
                <Check
                  label="Connect at launch"
                  registration={register("connectAtLaunch")}
                />
                <Check
                  label="Close window to tray"
                  registration={register("closeToTray")}
                />
                <Check
                  label="Block traffic when its client is down (fail closed)"
                  registration={register("failClosed")}
                />
              </div>
            </Fieldset>
          ) : null}
        </div>

        {issues.length > 0 ? (
          <ul
            className="mt-3 shrink-0 rounded-xl border border-danger/20 bg-danger/5 p-4 text-sm text-danger"
            role="alert"
          >
            {issues.map((issue) => (
              <li key={`${issue.field}-${issue.code}`}>
                {issue.field}: {issue.message}
              </li>
            ))}
          </ul>
        ) : null}

        <button
          disabled={actionPending || !isDirty}
          className="mt-3 inline-flex shrink-0 items-center gap-2 rounded-xl bg-brand px-5 py-3 font-semibold text-white disabled:opacity-50"
        >
          <Save size={18} aria-hidden /> Save settings
        </button>
      </form>
    </section>
  );
}

function TabButton({
  id,
  selected,
  controls,
  onSelect,
  children,
}: {
  id: string;
  selected: boolean;
  controls: string;
  onSelect: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      id={id}
      aria-selected={selected}
      aria-controls={controls}
      tabIndex={selected ? 0 : -1}
      onClick={onSelect}
      className={`flex-1 rounded-lg px-3 py-2 text-sm font-semibold ${
        selected
          ? "bg-surface text-brand shadow-sm"
          : "text-muted hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

function Fieldset({
  id,
  labelledBy,
  legend,
  children,
}: {
  id: string;
  labelledBy: string;
  legend: string;
  children: React.ReactNode;
}) {
  return (
    <fieldset
      id={id}
      role="tabpanel"
      aria-labelledby={labelledBy}
      className="grid gap-4 rounded-2xl border border-ink/10 bg-surface p-5 sm:grid-cols-2"
    >
      <legend className="px-2 font-semibold">{legend}</legend>
      {children}
    </fieldset>
  );
}

function Field({
  label,
  error,
  hint,
  className,
  children,
}: {
  label: string;
  error?: string;
  hint?: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={`block text-sm font-medium ${className ?? ""}`}>
      <label className="block">
        <span className="mb-1.5 block">{label}</span>
        {children}
        {error ? (
          <span className="mt-1 block text-xs text-danger">{error}</span>
        ) : null}
      </label>
      {hint ? (
        <p className="mt-1 text-xs font-normal text-muted">{hint}</p>
      ) : null}
    </div>
  );
}

function directDnsOptionLabel(preset: DirectDnsPreset, name: string): string {
  if (preset === "custom" || preset === "fake_ip") return name;
  return `${name} (${DIRECT_DNS_PRESET_SERVERS[preset].join(", ")})`;
}

function Check({
  label,
  registration,
}: {
  label: string;
  registration: UseFormRegisterReturn;
}) {
  return (
    <label className="flex items-center gap-3 text-sm font-medium">
      <input
        type="checkbox"
        {...registration}
        className="rounded border-ink/20 text-brand focus:ring-brand"
      />
      {label}
    </label>
  );
}
