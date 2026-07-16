import { expect, test } from "bun:test";
import { loadConfig } from "./index.ts";

test("configuration has safe local defaults", () => {
  const config = loadConfig({});
  expect(config.host).toBe("127.0.0.1");
  expect(config.apiPort).toBe(3001);
  expect(config.secureCookies).toBe(false);
  expect(config.eventPollMs).toBe(500);
});

test("invalid ports are rejected", () => {
  expect(() => loadConfig({ SOLOOPS_API_PORT: "70000" })).toThrow();
});

