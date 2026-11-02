import { describe, expect, it } from "vitest";
import {
  compareRoutes,
  formatRelative,
  interpolate,
  isIpv4,
  isIpv4Cidr,
  parseSignal,
  signalLevel,
  sortDefaultCandidates,
} from "./format.js";

describe("formatting helpers", () => {
  it("parses netsh signal text and maps it to bars", () => {
    expect(parseSignal("93%")).toBe(93);
    expect(parseSignal("信号 : 40 %")).toBe(40);
    expect(parseSignal(undefined)).toBeNull();
    expect(signalLevel(93)).toBe(4);
    expect(signalLevel(40)).toBe(2);
    expect(signalLevel(null)).toBe(0);
  });

  it("formats relative freshness", () => {
    const now = Date.parse("2026-11-02T10:00:00Z");
    expect(formatRelative("2026-11-02T09:59:30Z", "en", now)).toBe("30 seconds ago");
    expect(formatRelative("2026-11-02T09:58:00Z", "en", now)).toBe("2 minutes ago");
    expect(formatRelative(null, "en", now)).toBe("—");
  });

  it("validates IPv4 addresses and CIDR prefixes", () => {
    expect(isIpv4("192.168.1.1")).toBe(true);
    expect(isIpv4("256.1.1.1")).toBe(false);
    expect(isIpv4Cidr("10.0.0.0/8")).toBe(true);
    expect(isIpv4Cidr("10.0.0.0/33")).toBe(false);
    expect(isIpv4Cidr("10.0.0.0")).toBe(false);
  });

  it("interpolates named parameters and keeps unknown ones", () => {
    expect(interpolate("{a} of {b}", { a: 1, b: 2 })).toBe("1 of 2");
    expect(interpolate("{missing}", {})).toBe("{missing}");
  });

  it("sorts routes numerically by address and metric", () => {
    const routes = [
      { destinationPrefix: "10.0.0.0/8", effectiveMetric: 50 },
      { destinationPrefix: "9.0.0.0/8", effectiveMetric: 10 },
    ];
    expect([...routes].sort((a, b) => compareRoutes(a, b, "destination"))[0].destinationPrefix).toBe("9.0.0.0/8");
    expect([...routes].sort((a, b) => compareRoutes(a, b, "metric"))[0].effectiveMetric).toBe(10);
  });

  it("orders default-route candidates by family then effective metric", () => {
    const sorted = sortDefaultCandidates([
      { addressFamily: "IPv6", effectiveMetric: 1 },
      { addressFamily: "IPv4", effectiveMetric: 45 },
      { addressFamily: "IPv4", effectiveMetric: 35 },
    ]);
    expect(sorted.map((route) => route.effectiveMetric)).toEqual([35, 45, 1]);
  });
});
