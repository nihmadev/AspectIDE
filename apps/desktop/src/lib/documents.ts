import type { DocumentSnapshot } from "./types";
import { displayPath, normalizePath } from "./fileTree";

export function documentTitle(document: DocumentSnapshot) {
  return document.title || (document.path ? displayPath(document.path).split("/").pop() : null) || "Untitled";
}

export function documentDisplayPath(document: DocumentSnapshot) {
  return document.path ? displayPath(document.path) : document.title;
}

export function documentParentLabel(document: DocumentSnapshot) {
  if (!document.path) return "Unsaved editor";
  const normalized = displayPath(document.path);
  const index = normalized.lastIndexOf("/");
  return index === -1 ? normalized : normalized.slice(0, index);
}

export function documentRelativePath(document: DocumentSnapshot, root: string | null) {
  if (!document.path) return document.title;
  if (!root) return displayPath(document.path);
  const normalizedRoot = displayPath(root).replace(/\/+$/, "");
  const normalizedPath = displayPath(document.path);
  return normalizePath(normalizedPath).startsWith(`${normalizePath(normalizedRoot)}/`)
    ? normalizedPath.slice(normalizedRoot.length + 1)
    : normalizedPath;
}
