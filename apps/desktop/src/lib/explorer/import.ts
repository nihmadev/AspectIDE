import { luxCommands } from "./../tauri/commands";

/** Refuse single dropped files above this size — base64 IPC is fine for normal
 *  assets but a multi-GB drop would balloon memory in the webview. */
export const MAX_IMPORT_FILE_BYTES = 256 * 1024 * 1024;

/** True when the drag carries OS files (and is not one of our internal entry
 *  drags, which always set the application/x-lux-path marker). */
export function isExternalFileDrag(dataTransfer: DataTransfer | null): boolean {
  if (!dataTransfer) return false;
  const types = Array.from(dataTransfer.types ?? []);
  return types.includes("Files") && !types.includes("application/x-lux-path");
}

/** Snapshot dropped files synchronously — a DataTransfer is neutered after the
 *  drop handler returns, so callers must capture the list inside the handler. */
export function externalFilesFromDrop(dataTransfer: DataTransfer | null): File[] {
  if (!dataTransfer || dataTransfer.files.length === 0) return [];
  return Array.from(dataTransfer.files);
}

/**
 * Copies OS-dropped files into `targetDirectory` through the workspace-guarded
 * backend import command. Sequential on purpose: preserves drop order and keeps
 * peak memory to one file's bytes. Throws on the first failure with the file
 * name in the message so the explorer error strip says which one broke.
 */
export async function importExternalFiles(targetDirectory: string, files: readonly File[]): Promise<string[]> {
  const written: string[] = [];
  for (const file of files) {
    if (file.size > MAX_IMPORT_FILE_BYTES) {
      throw new Error(`${file.name}: file is too large to import (max ${Math.round(MAX_IMPORT_FILE_BYTES / (1024 * 1024))} MB)`);
    }
    const bytes = new Uint8Array(await file.arrayBuffer());
    const target = joinPath(targetDirectory, sanitizeDroppedName(file.name));
    try {
      written.push(await luxCommands.fsImportFile(target, toBase64(bytes)));
    } catch (error) {
      throw new Error(`${file.name}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  return written;
}

function joinPath(directory: string, name: string): string {
  const separator = directory.includes("\\") ? "\\" : "/";
  return directory.endsWith(separator) ? `${directory}${name}` : `${directory}${separator}${name}`;
}

/** Keeps just the leaf name and strips characters Windows forbids in names. */
function sanitizeDroppedName(name: string): string {
  const leaf = name.split(/[\\/]/).pop() ?? name;
  const cleaned = leaf.replace(/[<>:"|?*\u0000-\u001f]/g, "_").trim();
  return cleaned.length > 0 ? cleaned : "imported-file";
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const CHUNK = 0x8000;
  for (let index = 0; index < bytes.length; index += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(index, index + CHUNK));
  }
  return btoa(binary);
}
