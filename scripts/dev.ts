const commands = [
  ["bun", "run", "dev:api"],
  ["bun", "run", "dev:worker"],
  ["bun", "run", "dev:web"]
];

const children = commands.map((cmd) =>
  Bun.spawn(cmd, {
    cwd: process.cwd(),
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit"
  })
);

let stopping = false;
async function stop(signal: NodeJS.Signals = "SIGTERM") {
  if (stopping) return;
  stopping = true;
  for (const child of children) child.kill(signal);
  await Promise.allSettled(children.map((child) => child.exited));
}

process.on("SIGINT", () => void stop("SIGINT"));
process.on("SIGTERM", () => void stop("SIGTERM"));

const exits = await Promise.all(children.map((child) => child.exited));
await stop();
process.exit(exits.find((code) => code !== 0) ?? 0);

