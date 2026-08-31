export default async function globalTeardown() {
  let failure: Error | undefined;
  try {
    const response = await fetch("http://127.0.0.1:4173/__e2e/shutdown", {
      method: "POST",
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) throw new Error(`E2E harness shutdown returned HTTP ${response.status}`);
    const status = (await response.json()) as { ok: boolean; errors: string[] };
    if (!status.ok || status.errors.length > 0) {
      failure = new Error(`E2E harness reported background errors:\n${status.errors.join("\n")}`);
    }
  } catch (cause) {
    failure = cause instanceof Error ? cause : new Error("Could not read E2E harness status");
  }

  const deadline = Date.now() + 5_000;
  let stopped = false;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
    try {
      await fetch("http://127.0.0.1:4173/healthz", { signal: AbortSignal.timeout(15_000) });
    } catch {
      stopped = true;
      break;
    }
  }
  if (!stopped && !failure) failure = new Error("E2E harness did not stop within 5 seconds");

  if (failure) throw failure;
}
