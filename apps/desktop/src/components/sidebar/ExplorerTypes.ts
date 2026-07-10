import type { FsEntry } from '../../lib/types';

export type PendingCreate = {
  kind: "file" | "directory";
  parentPath: string;
};

export type PendingRename = {
  entry: FsEntry;
};

export type PendingDelete = {
  entry: FsEntry;
};

export type ClipboardEntry = {
  entry: FsEntry;
  operation: "copy" | "cut";
};

export type DraggedEntry = {
  entry: FsEntry;
};

export type ContextMenuState = {
  entry: FsEntry;
  source: "row" | "blank";
  x: number;
  y: number;
};

export type TreeAction = {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  onClick: () => void;
};
