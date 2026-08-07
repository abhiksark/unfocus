import { readFileSync } from "node:fs";

const exceptionFile = new URL("../.github/dependency-exceptions.json", import.meta.url);
const maximumLifetimeMs = 120 * 24 * 60 * 60 * 1_000;
const ecosystems = new Set(["javascript", "rust"]);

export function loadDependencyExceptions(now = new Date()) {
  const document = JSON.parse(readFileSync(exceptionFile, "utf8"));
  if (!Array.isArray(document.exceptions)) throw new Error("dependency exceptions must be an array");

  const identifiers = new Set();
  for (const exception of document.exceptions) {
    if (
      typeof exception.id !== "string" ||
      !ecosystems.has(exception.ecosystem) ||
      typeof exception.expires !== "string" ||
      typeof exception.reason !== "string" ||
      exception.reason.trim().length < 20
    ) {
      throw new Error("every dependency exception needs an id, known ecosystem, expiry, and reason");
    }
    if (identifiers.has(exception.id)) throw new Error(`duplicate exception: ${exception.id}`);
    identifiers.add(exception.id);

    if (!/^\d{4}-\d{2}-\d{2}$/.test(exception.expires)) {
      throw new Error(`invalid expiry for ${exception.id}`);
    }
    const expiry = new Date(`${exception.expires}T00:00:00Z`);
    if (
      Number.isNaN(expiry.valueOf()) ||
      expiry.toISOString().slice(0, 10) !== exception.expires
    ) {
      throw new Error(`invalid expiry for ${exception.id}`);
    }
    if (expiry <= now) throw new Error(`dependency exception expired: ${exception.id}`);
    if (expiry.valueOf() - now.valueOf() > maximumLifetimeMs) {
      throw new Error(`dependency exception exceeds 120 days: ${exception.id}`);
    }
  }

  return document.exceptions;
}

if (import.meta.main) {
  const exceptions = loadDependencyExceptions();
  console.log(`validated ${exceptions.length} expiring dependency exceptions`);
}
