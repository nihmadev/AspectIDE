import type { EditorGroup } from "./../store/index";
import type { DocumentSnapshot } from "./../types/index";

export function closedDocumentIdsForAllDocuments(openDocuments: DocumentSnapshot[]) {
  return openDocuments.map((document) => document.id);
}

export function closedDocumentIdsForOtherDocuments(openDocuments: DocumentSnapshot[], documentId: string) {
  return openDocuments.filter((document) => document.id !== documentId).map((document) => document.id);
}

export function closedDocumentIdsForEditorGroup(
  openDocuments: DocumentSnapshot[],
  editorGroups: EditorGroup[],
  groupId: string,
) {
  return closedDocumentIdsAfterRemovingGroup(openDocuments, editorGroups, groupId);
}

export function closedDocumentIdsForDocumentInGroup(
  openDocuments: DocumentSnapshot[],
  editorGroups: EditorGroup[],
  groupId: string,
  documentId: string,
) {
  return closedDocumentIdsAfterUpdatingGroup(openDocuments, editorGroups, groupId, (group) =>
    group.documentIds.filter((candidate) => candidate !== documentId),
  );
}

export function closedDocumentIdsForOtherDocumentsInGroup(
  openDocuments: DocumentSnapshot[],
  editorGroups: EditorGroup[],
  groupId: string,
  documentId: string,
) {
  return closedDocumentIdsAfterUpdatingGroup(openDocuments, editorGroups, groupId, (group) =>
    group.documentIds.includes(documentId) ? [documentId] : group.documentIds,
  );
}

export function closedDocumentIdsForDocumentsToRightInGroup(
  openDocuments: DocumentSnapshot[],
  editorGroups: EditorGroup[],
  groupId: string,
  documentId: string,
) {
  return closedDocumentIdsAfterUpdatingGroup(openDocuments, editorGroups, groupId, (group) => {
    const index = group.documentIds.indexOf(documentId);
    return index === -1 ? group.documentIds : group.documentIds.slice(0, index + 1);
  });
}

function closedDocumentIdsAfterRemovingGroup(
  openDocuments: DocumentSnapshot[],
  editorGroups: EditorGroup[],
  groupId: string,
) {
  return closedDocumentIdsForRemainingGroups(openDocuments, editorGroups.filter((group) => group.id !== groupId));
}

function closedDocumentIdsAfterUpdatingGroup(
  openDocuments: DocumentSnapshot[],
  editorGroups: EditorGroup[],
  groupId: string,
  updateDocumentIds: (group: EditorGroup) => string[],
) {
  const remainingGroups = editorGroups
    .map((group) => group.id === groupId ? { ...group, documentIds: updateDocumentIds(group) } : group)
    .filter((group) => group.documentIds.length > 0);
  return closedDocumentIdsForRemainingGroups(openDocuments, remainingGroups);
}

function closedDocumentIdsForRemainingGroups(openDocuments: DocumentSnapshot[], remainingGroups: EditorGroup[]) {
  const referencedDocumentIds = new Set(remainingGroups.flatMap((group) => group.documentIds));
  return openDocuments
    .filter((document) => !referencedDocumentIds.has(document.id))
    .map((document) => document.id);
}
