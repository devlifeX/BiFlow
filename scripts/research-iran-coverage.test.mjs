import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DOMAIN_SOURCES,
  cidrContains,
  domainCoveredBySuffixes,
  extractCidrsFromText,
  extractDirectSetFromPython,
  extractDomainsFromClashPayload,
  extractRipestatPrefixes,
  isCoveredByCidrs,
  parseCidr,
  researchIranCoverage,
} from "./research-iran-coverage.mjs";

test("Tehran Index harvest includes the company directory and remaining sectors", () => {
  const ids = DOMAIN_SOURCES.map((source) => source.id);
  assert.ok(ids.includes("tehranindex-companies"));
  assert.ok(ids.includes("tehranindex-gaming"));
  assert.ok(ids.includes("tehranindex-enterprise-software"));
  assert.ok(ids.includes("tehranindex-agri"));
  assert.equal(
    DOMAIN_SOURCES.find((source) => source.id === "tehranindex-companies")?.url,
    "https://tehranindex.com/companies",
  );
});

test("suffix rules cover subdomains and +.ir without listing every host", () => {
  const suffixes = ["ir", "arvancloud.com", "technolife.com"];
  assert.equal(domainCoveredBySuffixes("shop.example.ir", suffixes), true);
  assert.equal(domainCoveredBySuffixes("cdn.arvancloud.com", suffixes), true);
  assert.equal(domainCoveredBySuffixes("technolife.com", suffixes), true);
  assert.equal(domainCoveredBySuffixes("nottechnolife.com", suffixes), false);
  assert.equal(domainCoveredBySuffixes("example.com", suffixes), false);
});

test("CIDR containment skips a subnet already inside a supernet", () => {
  const existing = [parseCidr("185.143.232.0/22"), parseCidr("2a0b:4e00::/32")];
  assert.equal(isCoveredByCidrs("185.143.232.0/24", existing), true);
  assert.equal(isCoveredByCidrs("185.143.232.0/22", existing), true);
  assert.equal(isCoveredByCidrs("185.143.236.0/24", existing), false);
  assert.equal(isCoveredByCidrs("2a0b:4e00:1::/48", existing), true);
  assert.equal(
    cidrContains(parseCidr("10.0.0.0/8"), parseCidr("11.0.0.0/8")),
    false,
  );
});

test("parses clash, python direct, CIDR, and RIPEstat payloads", () => {
  assert.deepEqual(
    extractDomainsFromClashPayload(
      "payload:\n  - DOMAIN-SUFFIX,payping.io\n  - +.kavenegar.com\n  - DOMAIN-KEYWORD,ads\n",
    ),
    ["kavenegar.com", "payping.io"],
  );
  assert.deepEqual(
    extractDirectSetFromPython(
      'ads = {"tracker.com"}\ndirect = {\n    "hitobit.com",\n    "ewano.app",\n}\nproxy = {"blocked.com"}\n',
    ),
    ["ewano.app", "hitobit.com"],
  );
  assert.deepEqual(
    extractDirectSetFromPython(
      'custom_domains = {\n "proxy": ["blocked.com"],\n "direct": [\n "hitobit.com",\n "ewano.app",\n ],\n}\n',
    ),
    ["ewano.app", "hitobit.com"],
  );
  assert.deepEqual(
    extractCidrsFromText("# header\n185.143.232.0/22\nnot a prefix\n"),
    ["185.143.232.0/22"],
  );
  assert.deepEqual(
    extractRipestatPrefixes({
      data: { resources: { ipv4: ["5.22.0.0/16"], ipv6: ["2a00:d98::/32"] } },
    }),
    ["2a00:d98::/32", "5.22.0.0/16"],
  );
});

test("research dump diffs fetched hosts and CIDRs against the snapshot", async () => {
  const result = await researchIranCoverage({
    fetchDomain: async (url) => {
      if (url.includes("clash_rules_other")) {
        return {
          text: "payload:\n  - DOMAIN-SUFFIX,definitely-missing-example.com\n  - DOMAIN-SUFFIX,technolife.com\n",
          finalUrl: url,
        };
      }
      if (url.includes("custom_domains.py")) {
        return { text: 'direct = {\n    "azkivam.com",\n}\n', finalUrl: url };
      }
      return { text: '<a href="https://www.iran.ir/">iran</a>', finalUrl: url };
    },
    fetchIp: async (url) => ({
      text: url.includes("ips.txt") ? "185.143.232.0/24\n203.0.113.0/24\n" : "",
      finalUrl: url,
    }),
    fetchRipestat: async (url) => ({
      json: {
        data: {
          resources: { ipv4: ["5.22.0.0/17", "198.51.100.0/24"], ipv6: [] },
        },
      },
      finalUrl: url,
    }),
  });
  assert.ok(result.snapshot.iran_domains > 1_000);
  assert.ok(
    result.missingDomains.some((line) =>
      line.startsWith("definitely-missing-example.com"),
    ),
  );
  assert.ok(
    !result.missingDomains.some((line) => line.startsWith("technolife.com")),
  );
  assert.ok(
    result.missingCidrs.some((line) => line.startsWith("203.0.113.0/24")),
  );
  assert.ok(
    result.missingCidrs.some((line) => line.startsWith("198.51.100.0/24")),
  );
  assert.ok(
    !result.missingCidrs.some((line) => line.startsWith("5.22.0.0/17")),
  );
});
