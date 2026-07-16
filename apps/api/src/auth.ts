import type { FastifyReply, FastifyRequest } from "fastify";
import type { Owner } from "@soloops/contracts";
import { findSessionOwner, type SoloOpsDatabase } from "@soloops/db";
import { apiError } from "@soloops/server-shared";

export const SESSION_COOKIE = "soloops_session";

export function hashSessionToken(token: string): string {
  return new Bun.CryptoHasher("sha256").update(token).digest("hex");
}

export function generateSessionToken(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Buffer.from(bytes).toString("base64url");
}

export type AuthenticatedOwner = Owner & { sessionId: string };

export function sessionOwner(database: SoloOpsDatabase, request: FastifyRequest): AuthenticatedOwner | null {
  const token = request.cookies[SESSION_COOKIE];
  if (!token) return null;
  return findSessionOwner(database, hashSessionToken(token));
}

export async function requireOwner(
  database: SoloOpsDatabase,
  request: FastifyRequest,
  reply: FastifyReply
): Promise<AuthenticatedOwner | null> {
  const owner = sessionOwner(database, request);
  if (owner) return owner;
  await reply.code(401).send(apiError("unauthorized", "Owner session is required"));
  return null;
}

