import type { Plugin } from "vite";
import { WebSocketServer, type WebSocket } from "ws";
import type { IncomingMessage } from "node:http";
import { createState, listEvents } from "./state.ts";
import { createClients } from "./clients.ts";
import type { ScenarioDeps } from "./scenario.ts";
import { createMockRouter } from "./router.ts";

export function mockPlugin(): Plugin {
  return {
    name: "soloops-mock",
    apply: "serve",
    configureServer(server) {
      if (process.env.VITE_MOCK !== "1") return;

      const state = createState();
      const clients = createClients();
      const scenario: ScenarioDeps = { state, clients };
      const route = createMockRouter({ state, scenario });

      // REST 中间件
      server.middlewares.use(async (req, res, next) => {
        const url = req.url ?? "";
        if (
          !url.startsWith("/api/") &&
          !["/healthz", "/livez", "/readyz", "/metrics"].includes(url.split("?")[0])
        ) {
          return next();
        }
        let handled = false;
        try {
          handled = await route(req as IncomingMessage, res);
        } catch {
          if (!res.headersSent) {
            const payload = JSON.stringify({
              error: { code: "internal", message: "Mock request failed" }
            });
            res.statusCode = 500;
            res.setHeader("content-type", "application/json");
            res.setHeader("content-length", Buffer.byteLength(payload));
            res.end(payload);
          }
        }
        if (!handled && !res.headersSent) next();
      });

      // WebSocket 升级
      const wss = new WebSocketServer({ noServer: true });
      const httpServer = server.httpServer;
      if (!httpServer) return;
      httpServer.on("upgrade", (req, socket, head) => {
        const url = req.url ?? "";
        if (!url.startsWith("/api/events")) {
          return; // 交还默认行为
        }
        const parsed = new URL(url, "http://mock");
        const runId = parsed.searchParams.get("runId") ?? undefined;
        const after = Number(parsed.searchParams.get("after") ?? "0");
        wss.handleUpgrade(req, socket, head, (ws) => {
          const conn = ws as WebSocket;
          // 先补拉存量事件
          for (const evt of listEvents(state, after, runId)) {
            if (conn.readyState === conn.OPEN) conn.send(JSON.stringify(evt));
          }
          if (runId) clients.add(runId, conn);
          conn.on("close", () => clients.remove(conn));
        });
      });
    }
  };
}
