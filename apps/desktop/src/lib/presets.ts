export type PresetId =
  | "hiddify"
  | "openvpn"
  | "happ"
  | "v2rayn"
  | "nekoray"
  | "shadowsocks"
  | "wireguard"
  | "windscribe";

export type EgressKind = "local_proxy" | "owned_side_tunnel" | "unsupported";
export type PresetStatus = "working" | "catalog" | "unsupported";

export interface PresetDownloads {
  linux: string;
  windows: string;
}

export interface PresetSpec {
  id: PresetId;
  kind: EgressKind;
  status: PresetStatus;
  title: string;
  defaultPort: number | null;
  installHint: string;
  downloads: PresetDownloads;
}

export const PRESETS: PresetSpec[] = [
  {
    id: "hiddify",
    kind: "local_proxy",
    status: "working",
    title: "Hiddify",
    defaultPort: 12334,
    installHint: "Install Hiddify Next and keep its mixed port on loopback.",
    downloads: {
      linux: "https://github.com/hiddify/hiddify-app/releases/latest",
      windows: "https://github.com/hiddify/hiddify-app/releases/latest",
    },
  },
  {
    id: "openvpn",
    kind: "owned_side_tunnel",
    status: "working",
    title: "OpenVPN",
    defaultPort: null,
    installHint:
      "Install OpenVPN and choose a .ovpn profile. BiFlow never lets it take the default route.",
    downloads: {
      linux: "https://openvpn.net/community-downloads/",
      windows: "https://openvpn.net/community-downloads/",
    },
  },
  {
    id: "happ",
    kind: "local_proxy",
    status: "working",
    title: "Happ",
    defaultPort: 3067,
    installHint: "Run Happ and expose a local SOCKS or mixed port.",
    downloads: {
      linux: "https://www.happ.su/main/download",
      windows: "https://www.happ.su/main/download",
    },
  },
  {
    id: "v2rayn",
    kind: "local_proxy",
    status: "working",
    title: "v2rayN",
    defaultPort: 10808,
    installHint:
      "Run v2rayN and keep the local SOCKS port (default 10808) on loopback.",
    downloads: {
      linux: "https://github.com/2dust/v2rayN/releases/latest",
      windows: "https://github.com/2dust/v2rayN/releases/latest",
    },
  },
  {
    id: "nekoray",
    kind: "local_proxy",
    status: "working",
    title: "Nekoray",
    defaultPort: 2080,
    installHint: "Run Nekoray / NekoBox and expose its mixed SOCKS port.",
    downloads: {
      linux: "https://github.com/MatsuriDayo/nekoray/releases/latest",
      windows: "https://github.com/MatsuriDayo/nekoray/releases/latest",
    },
  },
  {
    id: "shadowsocks",
    kind: "local_proxy",
    status: "working",
    title: "Shadowsocks",
    defaultPort: 1080,
    installHint:
      "Run a local Shadowsocks client and point BiFlow at its SOCKS port.",
    downloads: {
      linux: "https://github.com/shadowsocks/shadowsocks-rust/releases/latest",
      windows:
        "https://github.com/shadowsocks/shadowsocks-windows/releases/latest",
    },
  },
  {
    id: "wireguard",
    kind: "owned_side_tunnel",
    status: "catalog",
    title: "WireGuard",
    defaultPort: null,
    installHint:
      "WireGuard will use the same side-tunnel driver as OpenVPN. The driver is not in this version.",
    downloads: {
      linux: "https://www.wireguard.com/install/",
      windows: "https://www.wireguard.com/install/",
    },
  },
  {
    id: "windscribe",
    kind: "owned_side_tunnel",
    status: "working",
    title: "Windscribe",
    defaultPort: null,
    installHint:
      "Generate an OpenVPN profile with your Windscribe service credentials at build.windscribe.com, then choose the .ovpn here. Do not run the Windscribe GUI at the same time.",
    downloads: {
      linux: "https://windscribe.com/getconfig/openvpn",
      windows: "https://windscribe.com/getconfig/openvpn",
    },
  },
];

export function presetById(id: PresetId): PresetSpec {
  return PRESETS.find((item) => item.id === id) ?? PRESETS[0]!;
}

/** Official vendor download page for the current desktop platform. */
export function downloadUrlFor(spec: PresetSpec, platform: string): string {
  return platform === "windows" ? spec.downloads.windows : spec.downloads.linux;
}
