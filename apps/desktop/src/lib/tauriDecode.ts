/**
 * IPC contract-validation seam.
 *
 * `invoke<T>()` is compile-time-only: `T` is erased at runtime, so a backend schema
 * drift or a malformed payload crosses the IPC boundary unchecked straight into
 * trusted frontend state. This module gives commands an opt-in decoder hook and a
 * single error path so validation can be adopted incrementally without a breaking
 * change to the hundreds of existing call sites.
 *
 * A decoder is `(raw: unknown) => T`: it normalizes/validates a payload and throws
 * (ideally a `CommandContractError`) when the shape is wrong. Commands that pass a
 * decoder to `invokeRequired`/`invokeOptional` get runtime validation; commands
 * that don't are unchanged.
 */

export type IpcDecoder<T> = (raw: unknown) => T;

/** Raised when an IPC payload fails its decoder. Carries the command name so the
 *  central handler can log/report with full context. */
export class CommandContractError extends Error {
  constructor(
    readonly command: string,
    readonly detail: string,
    readonly raw: unknown,
  ) {
    super(`IPC contract violation for "${command}": ${detail}`);
    this.name = "CommandContractError";
  }
}

/** Central sink for contract violations. Logged once here so every command shares
 *  the same diagnostics surface; the error is then rethrown for the caller. */
export function reportCommandContractError(error: CommandContractError): void {
  console.error(`[lux] ${error.message}`, { raw: error.raw });
}

/** Runs `decode` against a payload, routing any failure through the central error
 *  path and rethrowing a normalized `CommandContractError`. */
export function decodeIpcResult<T>(command: string, raw: unknown, decode: IpcDecoder<T>): T {
  try {
    return decode(raw);
  } catch (error) {
    const contractError = error instanceof CommandContractError
      ? error
      : new CommandContractError(command, error instanceof Error ? error.message : String(error), raw);
    reportCommandContractError(contractError);
    throw contractError;
  }
}

// ── Primitive decoder building blocks ──

/** Asserts the payload is an array, returning it untouched. Cheap, unambiguous
 *  shape guard for list-returning commands (e.g. the file-tree reads that feed the
 *  explorer) — "expected an array, got X" is always a real backend contract break. */
export function expectArray<T = unknown>(raw: unknown): T[] {
  if (!Array.isArray(raw)) throw new Error(`expected an array, received ${describe(raw)}`);
  return raw as T[];
}

/** Asserts the payload is a non-null object (not an array). */
export function expectObject(raw: unknown): Record<string, unknown> {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
    throw new Error(`expected an object, received ${describe(raw)}`);
  }
  return raw as Record<string, unknown>;
}

function describe(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}
