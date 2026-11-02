export const formatTime = (value, language) =>
  value
    ? new Intl.DateTimeFormat(language, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(value))
    : "—";

export function formatRelative(value, language, now = Date.now()) {
  if (!value) return "—";
  const seconds = Math.round((new Date(value).getTime() - now) / 1000);
  if (!Number.isFinite(seconds)) return "—";
  const format = new Intl.RelativeTimeFormat(language, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 5) return format.format(0, "second");
  if (abs < 60) return format.format(seconds, "second");
  if (abs < 3600) return format.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return format.format(Math.round(seconds / 3600), "hour");
  return format.format(Math.round(seconds / 86400), "day");
}

export const errorMessage = (error) =>
  error?.message || error?.code || String(error);

export const autoGuidKey = (value) =>
  String(value || "")
    .replace(/[{}]/g, "")
    .trim()
    .toLowerCase();

export const autoResultText = (value) => {
  if (value == null || value === "") return "—";
  if (typeof value === "string") return value;
  return value.message || value.summary || JSON.stringify(value);
};

export function parseSignal(value) {
  if (typeof value === "number") return Math.max(0, Math.min(100, value));
  const match = String(value ?? "").match(/(\d{1,3})\s*%/);
  return match ? Math.min(100, Number(match[1])) : null;
}

export function signalLevel(percent) {
  if (percent == null) return 0;
  if (percent >= 80) return 4;
  if (percent >= 60) return 3;
  if (percent >= 35) return 2;
  return percent > 0 ? 1 : 0;
}

export const interpolate = (template, params = {}) =>
  String(template).replace(/\{(\w+)\}/g, (match, key) =>
    params[key] == null ? match : String(params[key]),
  );

const OCTET = "(25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)";
const IPV4 = new RegExp(`^${OCTET}(\\.${OCTET}){3}$`);

export const isIpv4 = (value) => IPV4.test(String(value || "").trim());

export function isIpv4Cidr(value) {
  const [address, prefix, extra] = String(value || "").trim().split("/");
  return (
    extra === undefined &&
    isIpv4(address) &&
    /^\d{1,2}$/.test(prefix || "") &&
    Number(prefix) <= 32
  );
}

/** A plain IPv6 literal (no zone index); validated again in Rust. */
export function isIpv6(value) {
  const text = String(value || "").trim();
  if (!/^[0-9a-f:]+$/i.test(text) || !text.includes(":")) return false;
  const halves = text.split("::");
  if (halves.length > 2) return false;
  const groups = halves.flatMap((half) => (half ? half.split(":") : []));
  if (groups.some((group) => group.length < 1 || group.length > 4)) return false;
  return halves.length === 2 ? groups.length < 8 : groups.length === 8;
}

export function isIpv6Cidr(value) {
  const [address, prefix, extra] = String(value || "").trim().split("/");
  return extra === undefined && isIpv6(address) && /^\d{1,3}$/.test(prefix || "") && Number(prefix) <= 128;
}

export const isDefaultPrefix = (prefix) =>
  prefix === "0.0.0.0/0" || prefix === "::/0";

export const routeIdentity = (route) =>
  `${route.addressFamily}|${route.destinationPrefix}|${route.nextHop}|${route.interfaceIndex}`;

const ipSortKey = (value) => {
  const text = String(value || "");
  const [address, prefix = ""] = text.split("/");
  if (isIpv4(address))
    return (
      address
        .split(".")
        .map((part) => part.padStart(3, "0"))
        .join(".") + `/${prefix.padStart(3, "0")}`
    );
  return `~${text.toLowerCase()}`;
};

export function compareRoutes(a, b, key) {
  switch (key) {
    case "metric": {
      const left = a.effectiveMetric ?? a.routeMetric ?? Infinity;
      const right = b.effectiveMetric ?? b.routeMetric ?? Infinity;
      return left - right;
    }
    case "nextHop":
      return ipSortKey(a.nextHop).localeCompare(ipSortKey(b.nextHop));
    case "interface":
      return String(a.interfaceAlias || "").localeCompare(
        String(b.interfaceAlias || ""),
      );
    case "type":
      return Number(b.persistent) - Number(a.persistent);
    case "family":
      return String(a.addressFamily).localeCompare(String(b.addressFamily));
    default:
      return ipSortKey(a.destinationPrefix).localeCompare(
        ipSortKey(b.destinationPrefix),
      );
  }
}

export const isAdapterDisabled = (item) =>
  String(item?.adminStatus || "").toLowerCase() === "down";

export const isAdapterOnline = (item) =>
  !isAdapterDisabled(item) && Boolean(item?.wifi?.connected || item?.defaultRoute);

export const hasManualMetric = (item) =>
  Boolean(item?.deviations?.manualIpv4Metric || item?.deviations?.manualIpv6Metric);

export const primaryIpv4 = (item) => {
  const address = (item?.ipv4Addresses || [])[0];
  return address ? `${address.ipAddress}/${address.prefixLength}` : null;
};

export function sortAdapters(items) {
  return [...items].sort(
    (a, b) =>
      Number(isAdapterOnline(b)) - Number(isAdapterOnline(a)) ||
      Number(isAdapterDisabled(a)) - Number(isAdapterDisabled(b)) ||
      String(a.alias).localeCompare(String(b.alias)),
  );
}

export function sortDefaultCandidates(routes) {
  return [...routes].sort(
    (a, b) =>
      String(a.addressFamily).localeCompare(String(b.addressFamily)) ||
      (a.effectiveMetric ?? a.routeMetric ?? Infinity) -
        (b.effectiveMetric ?? b.routeMetric ?? Infinity),
  );
}
