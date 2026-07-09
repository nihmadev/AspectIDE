export const LUX_EDITOR_TAB_MIME = "application/x-lux-editor-tab";

export function setEditorTabDragData(dataTransfer: DataTransfer, documentId: string) {
  dataTransfer.setData(LUX_EDITOR_TAB_MIME, documentId);
  dataTransfer.effectAllowed = "copy";
}

export function readEditorTabDrop(dataTransfer: DataTransfer) {
  const documentId = dataTransfer.getData(LUX_EDITOR_TAB_MIME).trim();
  return documentId || null;
}

export function dragEventHasEditorTab(dataTransfer: DataTransfer) {
  return dataTransfer.types.includes(LUX_EDITOR_TAB_MIME);
}