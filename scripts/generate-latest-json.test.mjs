import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import {
  buildLatestJson,
  discoverSignedArtifacts,
  generateLatestJsonFromDirectory,
  normalizeVersion,
  validateLatestJson,
} from "./generate-latest-json.mjs";

const FAKE_SIG =
  "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTVGVzdEtleVNpZ25hdHVyZQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzAwMDAwMDAwCWZpbGU6dGVzdC5idW5kbGUKdGVzdA==";

function writeFakeBundle(directory, name, signature = FAKE_SIG) {
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, name), `bundle:${name}`, "utf8");
  writeFileSync(join(directory, `${name}.sig`), `${signature}\n`, "utf8");
}

describe("generate-latest-json", () => {
  it("normalizes release tags to bare semver", () => {
    assert.equal(normalizeVersion("v1.2.0"), "1.2.0");
    assert.equal(normalizeVersion("1.2.0"), "1.2.0");
  });

  it("builds a signed manifest for deb, AppImage, and NSIS", () => {
    const manifest = buildLatestJson(
      "devlifeX/BiFlow",
      "v1.2.0",
      [
        {
          platform: "linux-deb-x86_64",
          bundlePath: "/tmp/BiFlow_1.2.0_amd64.deb",
          fileName: "BiFlow_1.2.0_amd64.deb",
          signature: FAKE_SIG,
        },
        {
          platform: "linux-x86_64",
          bundlePath: "/tmp/BiFlow_1.2.0_amd64.AppImage",
          fileName: "BiFlow_1.2.0_amd64.AppImage",
          signature: FAKE_SIG,
        },
        {
          platform: "windows-x86_64",
          bundlePath: "/tmp/BiFlow_1.2.0_x64-setup.exe",
          fileName: "BiFlow_1.2.0_x64-setup.exe",
          signature: FAKE_SIG,
        },
      ],
      { notes: "Signed updater test", pubDate: "2026-08-13T12:00:00.000Z" },
    );

    assert.equal(manifest.version, "1.2.0");
    assert.equal(manifest.notes, "Signed updater test");
    assert.equal(
      manifest.platforms["linux-deb-x86_64"].url,
      "https://github.com/devlifeX/BiFlow/releases/download/v1.2.0/BiFlow_1.2.0_amd64.deb",
    );
    assert.equal(
      manifest.platforms["linux-x86_64"].url,
      "https://github.com/devlifeX/BiFlow/releases/download/v1.2.0/BiFlow_1.2.0_amd64.AppImage",
    );
    assert.equal(manifest.platforms["windows-x86_64"].signature, FAKE_SIG);
    validateLatestJson(manifest);
  });

  it("rejects malformed manifests and signature URLs", () => {
    assert.throws(
      () => validateLatestJson({ version: "not-a-version" }),
      /semver/,
    );
    assert.throws(
      () =>
        validateLatestJson({
          version: "1.2.0",
          notes: "x",
          pub_date: "2026-08-13T12:00:00.000Z",
          platforms: {
            "linux-deb-x86_64": {
              url: "https://github.com/devlifeX/BiFlow/releases/download/v1.2.0/app.deb",
              signature: FAKE_SIG,
            },
            "linux-x86_64": {
              url: "https://github.com/devlifeX/BiFlow/releases/download/v1.2.0/app.AppImage",
              signature: "https://example.com/app.AppImage.sig",
            },
          },
        }),
      /must not be a URL/,
    );
  });

  it("discovers fake signed artifacts from a staging directory", () => {
    const directory = mkdtempSync(join(tmpdir(), "biflow-latest-json-"));
    writeFakeBundle(directory, "BiFlow_1.2.0_amd64.deb");
    writeFakeBundle(directory, "BiFlow_1.2.0_amd64.AppImage");
    writeFakeBundle(directory, "BiFlow_1.2.0_x64-setup.exe");

    const artifacts = discoverSignedArtifacts(directory);
    assert.equal(artifacts.length, 3);
    assert.deepEqual(artifacts.map((artifact) => artifact.platform).sort(), [
      "linux-deb-x86_64",
      "linux-x86_64",
      "windows-x86_64",
    ]);

    const manifest = generateLatestJsonFromDirectory(
      directory,
      "devlifeX/BiFlow",
      "v1.2.0",
    );
    assert.equal(manifest.version, "1.2.0");
    validateLatestJson(manifest);
    assert.match(
      readFileSync(join(directory, "BiFlow_1.2.0_amd64.AppImage"), "utf8"),
      /bundle:/,
    );
  });

  it("discovers artifacts through the layout download-artifact preserves", () => {
    const directory = mkdtempSync(join(tmpdir(), "biflow-latest-json-nested-"));
    writeFakeBundle(
      join(directory, "appimage"),
      "BiFlow_1.2.16_amd64.AppImage",
    );
    writeFakeBundle(
      join(directory, "target", "release", "bundle", "nsis"),
      "BiFlow_1.2.16_x64-setup.exe",
    );
    writeFileSync(join(directory, "BiFlow.exe"), "portable", "utf8");
    writeFakeBundle(join(directory, "deb"), "BiFlow_1.2.16_amd64.deb");

    const manifest = generateLatestJsonFromDirectory(
      directory,
      "devlifeX/BiFlow",
      "v1.2.16",
    );
    assert.equal(
      manifest.platforms["linux-deb-x86_64"].url,
      "https://github.com/devlifeX/BiFlow/releases/download/v1.2.16/BiFlow_1.2.16_amd64.deb",
    );
    assert.equal(
      manifest.platforms["linux-x86_64"].url,
      "https://github.com/devlifeX/BiFlow/releases/download/v1.2.16/BiFlow_1.2.16_amd64.AppImage",
    );
    assert.equal(
      manifest.platforms["windows-x86_64"].url,
      "https://github.com/devlifeX/BiFlow/releases/download/v1.2.16/BiFlow_1.2.16_x64-setup.exe",
    );
    validateLatestJson(manifest);
  });

  it("rejects duplicate bundles for one platform", () => {
    const directory = mkdtempSync(join(tmpdir(), "biflow-latest-json-dupe-"));
    writeFakeBundle(
      join(directory, "appimage"),
      "BiFlow_1.2.16_amd64.AppImage",
    );
    writeFakeBundle(join(directory, "stale"), "BiFlow_1.2.15_amd64.AppImage");
    assert.throws(
      () => discoverSignedArtifacts(directory),
      /expected one \.AppImage bundle, found 2/,
    );
  });

  it("names the staged files when a platform is missing", () => {
    const directory = mkdtempSync(join(tmpdir(), "biflow-latest-json-report-"));
    writeFakeBundle(
      join(directory, "appimage"),
      "BiFlow_1.2.16_amd64.AppImage",
    );
    assert.throws(
      () =>
        generateLatestJsonFromDirectory(
          directory,
          "devlifeX/BiFlow",
          "v1.2.16",
        ),
      /appimage[/\\]BiFlow_1\.2\.16_amd64\.AppImage/,
    );
  });

  it("requires all three signed updater packages before publishing", () => {
    const directory = mkdtempSync(
      join(tmpdir(), "biflow-latest-json-missing-"),
    );
    mkdirSync(directory, { recursive: true });
    writeFakeBundle(directory, "BiFlow_1.2.0_amd64.AppImage");
    assert.throws(
      () =>
        generateLatestJsonFromDirectory(directory, "devlifeX/BiFlow", "v1.2.0"),
      /expected 3 signed updater artifacts/,
    );
  });

  it("rejects a stale package name even when its signature is present", () => {
    assert.throws(
      () =>
        buildLatestJson("devlifeX/BiFlow", "v1.2.0", [
          {
            platform: "linux-deb-x86_64",
            bundlePath: "/tmp/BiFlow_1.1.9_amd64.deb",
            fileName: "BiFlow_1.1.9_amd64.deb",
            signature: FAKE_SIG,
          },
        ]),
      /unexpected linux-deb-x86_64 asset/,
    );
  });
});
