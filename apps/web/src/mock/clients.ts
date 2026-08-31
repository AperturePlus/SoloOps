import type WebSocket from "ws";

export interface MockClients {
  add(runId: string, socket: WebSocket): void;
  remove(socket: WebSocket): void;
  broadcast(runId: string, message: string): void;
  size(): number;
}

export function createClients(): MockClients {
  const byRun = new Map<string, Set<WebSocket>>();

  return {
    add(runId, socket) {
      let set = byRun.get(runId);
      if (!set) {
        set = new Set();
        byRun.set(runId, set);
      }
      set.add(socket);
    },
    remove(socket) {
      for (const set of byRun.values()) set.delete(socket);
    },
    broadcast(runId, message) {
      const set = byRun.get(runId);
      if (!set) return;
      for (const socket of set) {
        if (socket.readyState === socket.OPEN) socket.send(message);
      }
    },
    size() {
      let total = 0;
      for (const set of byRun.values()) total += set.size;
      return total;
    }
  };
}
