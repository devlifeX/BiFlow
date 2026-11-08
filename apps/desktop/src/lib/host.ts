/**
 * Reduces anything a user might paste to the host a routing rule matches on.
 *
 * The rule parser rejects URLs, so `https://www.example.ir/` has to become
 * `www.example.ir` before it reaches the backend. Everything that is not a host —
 * scheme, credentials, port, path, query, fragment — is dropped.
 */
export function extractHost(input: string): string {
  let value = input.trim();
  if (value === "") return "";

  // Strip the scheme, or a protocol-relative `//host` prefix.
  value = value.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/^\/\//, "");
  // Anything after the authority is not part of the host.
  value = value.split(/[/?#\\]/, 1)[0] ?? "";
  // user:password@host
  const at = value.lastIndexOf("@");
  if (at !== -1) value = value.slice(at + 1);

  if (value.startsWith("[")) {
    // Bracketed IPv6 keeps its colons; the port lives outside the bracket.
    const close = value.indexOf("]");
    if (close !== -1) return value.slice(1, close).toLowerCase();
  }
  // A bare IPv6 address has several colons and no port to strip.
  if ((value.match(/:/g) ?? []).length <= 1) {
    value = value.split(":", 1)[0] ?? "";
  }

  // A trailing dot is a valid FQDN root but never matches a rule.
  return value.replace(/\.+$/, "").toLowerCase();
}
