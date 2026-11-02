import { html } from "lit";

// Single-path, stroke-based icons on a 24px grid (Fluent-style outline).
const CIRCLE = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z";
const SHIELD = "M12 3l8 3v6c0 4.5-3.4 8-8 9-4.6-1-8-4.5-8-9V6z";

const paths = {
  overview: "M3 11.5 12 4l9 7.5M5.5 9.5V20h5v-5.5h3V20h5V9.5",
  adapters:
    "M9 3h6v5H9zM12 8v4M5 12h14M5 12v4M12 12v4M19 12v4M3 16h4v5H3zM10 16h4v5h-4zM17 16h4v5h-4z",
  routes: "M12 3v3M12 12v2M12 20v1M5 6h11l3 3-3 3H5zM19 14H8l-3 3 3 3h11z",
  profiles: "M3 4h18v4H3zM5 8v12h14V8M10 12h4",
  diagnostics: "M3 12h4l2-6 4 12 2-6h6",
  maintenance: "M9 14 4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3",
  settings:
    "M4 6h9M17 6h3M15 4v4M4 12h3M11 12h9M9 10v4M4 18h11M19 18h1M17 16v4",
  refresh: "M20 12a8 8 0 1 1-2.34-5.66L20 8.5M20 3.5v5h-5",
  wifi: "M2 8.5a15 15 0 0 1 20 0M5 12a10 10 0 0 1 14 0M8.5 15.5a5 5 0 0 1 7 0M12 19h.01",
  ethernet: "M6 4h12v10h-3v3H9v-3H6zM9 8v2M12 8v2M15 8v2M12 17v4",
  vpn: `${SHIELD}M9.5 11.5h5v4h-5zM10.5 11.5V10a1.5 1.5 0 0 1 3 0v1.5`,
  shield: `${SHIELD}M9 12l2 2 4-4`,
  virtual: "M12 3l8 4.5v9L12 21l-8-4.5v-9zM4 7.5l8 4.5 8-4.5M12 12v9",
  loopback:
    "M17 2l3 3-3 3M20 5H9a5 5 0 0 0-5 5M7 22l-3-3 3-3M4 19h11a5 5 0 0 0 5-5",
  tunnel: "M3 20V11a9 9 0 0 1 18 0v9M7 20v-8a5 5 0 0 1 10 0v8",
  globe: `${CIRCLE}M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z`,
  check: "M5 12.5l4.5 4.5L19 7.5",
  success: `${CIRCLE}M8 12.5l2.5 2.5L16 9.5`,
  warning: "M12 3.5 2.5 20h19zM12 10v4.5M12 17.5h.01",
  error: `${CIRCLE}M9 9l6 6M15 9l-6 6`,
  info: `${CIRCLE}M12 11v5M12 8h.01`,
  copy: "M9 9h11v11H9zM5 15H4V4h11v1",
  edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
  pin: "M9 4h6l-1 6 4 4H6l4-4zM12 14v7",
  close: "M6 6l12 12M18 6 6 18",
  chevronDown: "M6 9l6 6 6-6",
  chevronUp: "M6 15l6-6 6 6",
  chevronRight: "M9 6l6 6-6 6",
  plus: "M12 5v14M5 12h14",
  activity: "M6 16V11a6 6 0 0 1 12 0v5l2 2H4zM10 20a2 2 0 0 0 4 0",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4",
  power: "M12 3v8M7 6.5a7 7 0 1 0 10 0",
  clock: `${CIRCLE}M12 7v5l3 2`,
  save: "M12 4v11M7 10l5 5 5-5M5 20h14",
  arrowRight: "M5 12h14M13 6l6 6-6 6",
  user: "M12 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM4 21a8 8 0 0 1 16 0",
  other: `${CIRCLE}M8 12h.01M12 12h.01M16 12h.01`,
};

export function icon(name, className = "") {
  return html`<svg
    class="icon ${className}"
    viewBox="0 0 24 24"
    aria-hidden="true"
    focusable="false"
  ><path d=${paths[name] || paths.other}></path></svg>`;
}

export function adapterIconName(item) {
  if (item?.wifi) return "wifi";
  const type = String(item?.type || "").toLowerCase();
  if (type.includes("wi-fi") || type.includes("wifi")) return "wifi";
  if (type.includes("ethernet")) return "ethernet";
  if (type.includes("vpn")) return "vpn";
  if (type.includes("tunnel")) return "tunnel";
  if (type.includes("virtual")) return "virtual";
  if (type.includes("loopback")) return "loopback";
  return "other";
}
