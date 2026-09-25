import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const script = join(root, "scripts/list-xwin-payloads.py");

const fixture = {
  packages: [
    {
      id: "Microsoft.VisualStudio.Product.BuildTools",
      dependencies: {
        "Microsoft.VisualStudio.Component.VC.14.28.x86.x64": "1.0",
        "Microsoft.VisualStudio.Component.VC.14.29.16.10.x86.x64": "1.0",
      },
    },
    {
      id: "Microsoft.VC.14.29.16.10.CRT.Headers.base",
      payloads: [
        {
          fileName: "Microsoft.VC.14.29.16.10.CRT.Headers.base.vsix",
          sha256: "aa".repeat(32),
          url: "https://example.test/headers.vsix",
          size: 10,
        },
      ],
    },
    {
      id: "Microsoft.VC.14.29.16.10.CRT.x64.Desktop.base",
      payloads: [
        {
          fileName: "Microsoft.VC.14.29.16.10.CRT.x64.Desktop.base.vsix",
          sha256: "bb".repeat(32),
          url: "https://example.test/desktop.vsix",
          size: 44532816,
        },
      ],
    },
    {
      id: "Microsoft.VC.14.29.16.10.CRT.x64.Store.base",
      payloads: [
        {
          fileName: "Microsoft.VC.14.29.16.10.CRT.x64.Store.base.vsix",
          sha256: "cc".repeat(32),
          url: "https://example.test/store.vsix",
          size: 20,
        },
      ],
    },
    {
      id: "Win11SDK_10.0.22621",
      payloads: [
        {
          fileName: "Installers\\Windows SDK Desktop Headers x86-x86_en-us.msi",
          sha256: "dd".repeat(32),
          url: "https://example.test/sdk-headers.msi",
          size: 1,
        },
        {
          fileName:
            "Installers\\Windows SDK for Windows Store Apps Headers-x86_en-us.msi",
          sha256: "ee".repeat(32),
          url: "https://example.test/sdk-store-headers.msi",
          size: 1,
        },
        {
          fileName: "Installers\\Windows SDK Desktop Headers x64-x86_en-us.msi",
          sha256: "ff".repeat(32),
          url: "https://example.test/sdk-x64-headers.msi",
          size: 1,
        },
        {
          fileName: "Installers\\Windows SDK Desktop Libs x64-x86_en-us.msi",
          sha256: "11".repeat(32),
          url: "https://example.test/sdk-libs.msi",
          size: 1,
        },
        {
          fileName:
            "Installers\\Windows SDK for Windows Store Apps Libs-x86_en-us.msi",
          sha256: "22".repeat(32),
          url: "https://example.test/sdk-store-libs.msi",
          size: 1,
        },
      ],
    },
    {
      id: "Microsoft.Windows.UniversalCRT.HeadersLibsSources.Msi",
      payloads: [
        {
          fileName: "Universal CRT Headers Libraries and Sources-x86_en-us.msi",
          sha256: "33".repeat(32),
          url: "https://example.test/ucrt.msi",
          size: 1,
        },
        {
          fileName: "Installers\\16ab2ea2187acffa6435e334796c8c89.cab",
          sha256: "44".repeat(32),
          url: "https://example.test/ucrt.cab",
          size: 139000000,
        },
      ],
    },
  ],
};

describe("list-xwin-payloads", () => {
  it("emits cargo-xwin cache names for CRT Desktop vsix and ucrt cabs", () => {
    const dir = mkdtempSync(join(tmpdir(), "xwin-payloads-"));
    const vsman = join(dir, "pkg.vsman");
    writeFileSync(vsman, JSON.stringify(fixture));
    const python = process.platform === "win32" ? "py" : "python3";
    const pythonArgs = process.platform === "win32" ? ["-3"] : [];
    const output = execFileSync(
      python,
      [...pythonArgs, script, vsman, "x86_64"],
      {
        encoding: "utf8",
      },
    );
    const files = output
      .trim()
      .split("\n")
      .map((line) => line.split("\t")[1]);
    assert.ok(
      files.includes("Microsoft.VC.14.29.16.10.CRT.x64.Desktop.base.vsix"),
    );
    assert.ok(files.includes("ucrt.msi"));
    assert.ok(files.includes("ucrt/16ab2ea2187acffa6435e334796c8c89.cab"));
    assert.ok(files.includes("Win11SDK_10.0.22621_headers.msi"));
    assert.ok(files.includes("Win11SDK_10.0.22621_libs_x86_64.msi"));
    assert.equal(
      files.filter((name) => name.includes("14.28")).length,
      0,
      "must pick the latest CRT version from BuildTools",
    );
  });
});
