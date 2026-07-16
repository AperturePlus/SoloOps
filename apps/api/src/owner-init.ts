import { closeDatabase, countUsers, createOwner, openDatabase, runMigrations } from "@soloops/db";
import { loadConfig } from "@soloops/server-shared";

async function readSecret(label: string): Promise<string> {
  if (!process.stdin.isTTY || !process.stdin.setRawMode) throw new Error("Owner initialization requires an interactive terminal");
  process.stdout.write(label);
  process.stdin.setRawMode(true);
  process.stdin.resume();
  process.stdin.setEncoding("utf8");
  return new Promise((resolve, reject) => {
    let value = "";
    const cleanup = () => {
      process.stdin.off("data", onData);
      process.stdin.setRawMode(false);
      process.stdin.pause();
      process.stdout.write("\n");
    };
    const onData = (data: string) => {
      if (data === "\u0003") {
        cleanup();
        reject(new Error("Owner initialization cancelled"));
      } else if (data === "\r" || data === "\n") {
        cleanup();
        resolve(value);
      } else if (data === "\u007f" || data === "\b") {
        if (value) {
          value = value.slice(0, -1);
          process.stdout.write("\b \b");
        }
      } else if (/^[\x20-\x7E]+$/.test(data)) {
        value += data;
        process.stdout.write("*".repeat(data.length));
      }
    };
    process.stdin.on("data", onData);
  });
}

const config = loadConfig();
const database = openDatabase(config.databasePath);
try {
  runMigrations(database);
  if (countUsers(database) > 0) throw new Error("Owner already exists");
  const username = (prompt("Owner username [owner]:") ?? "").trim() || "owner";
  const password = await readSecret("Owner password: ");
  const confirmation = await readSecret("Confirm password: ");
  if (password !== confirmation) throw new Error("Passwords do not match");
  if (password.length < 12) throw new Error("Password must be at least 12 characters");
  const passwordHash = await Bun.password.hash(password, { algorithm: "argon2id" });
  const owner = createOwner(database, username, passwordHash);
  console.log(`Owner '${owner.username}' created.`);
} finally {
  closeDatabase(database);
}

