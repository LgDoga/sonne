import { test } from "node:test";
import assert from "node:assert/strict";
import {
  isFullAttachmentComparisonRequest,
  resolveConversationAttachmentReferences,
  type AttachmentCandidate,
  type HistoricalMessageLike,
} from "../src/attachmentResolver.ts";

const file = (id: string, chatId: string, messageId: string, fileName: string, position = 0): AttachmentCandidate => ({
  id,
  chatId,
  messageId,
  attachmentVersionId: `version-${id}`,
  versionNumber: 1,
  fileName,
  extension: fileName.split(".").pop() ?? "",
  mimeType: fileName.endsWith(".pdf") ? "application/pdf" : "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  originalSize: 4096,
  purpose: "message",
  position,
});

function message(dbId: string, chatId: string, attachments: AttachmentCandidate[]): HistoricalMessageLike {
  return { dbId, role: "user", attachments: attachments.map((item) => ({ ...item, chatId })) };
}

const history: HistoricalMessageLike[] = [
  message("m1", "chat-a", [file("a", "chat-a", "m1", "Requirements.pdf")]),
  { dbId: "a1", role: "assistant", attachments: [] },
  message("m2", "chat-a", [file("b", "chat-a", "m2", "Report.docx")]),
];

const resolve = (prompt: string, items = history) => resolveConversationAttachmentReferences({
  prompt,
  chatId: "chat-a",
  history: items,
});

test("resolves the latest prior PDF by explicit type and previous cue", () => {
  const result = resolve("Порівняй із попереднім PDF");
  assert.equal(result.status, "resolved");
  assert.equal(result.references.length, 1);
  assert.equal(result.references[0].attachment.fileName, "Requirements.pdf");
  assert.equal(result.references[0].messageId, "m1");
});

test("resolves an explicitly named historical attachment", () => {
  const result = resolve("Перевір відповідність вимогам із Requirements.pdf");
  assert.equal(result.status, "resolved");
  assert.equal(result.references[0].attachment.id, "a");
});

test("preserves the exact original file size for historical local reads and Vision", () => {
  const result = resolve("Перевір відповідність вимогам із Requirements.pdf");
  assert.equal(result.status, "resolved");
  assert.equal(result.references[0].attachment.originalSize, 4096);
});

test("does not resolve references from a different chat", () => {
  const foreign = [message("foreign-message", "chat-b", [file("foreign", "chat-b", "foreign-message", "Other.pdf")])];
  const result = resolve("Використай попередній PDF", foreign);
  assert.equal(result.status, "not-found");
  assert.equal(result.references.length, 0);
});

test("rejects an attachment whose message ownership does not match", () => {
  const malformed = [message("m1", "chat-a", [file("bad", "chat-a", "different-message", "Bad.pdf")])];
  const result = resolve("Візьми попередній PDF", malformed);
  assert.equal(result.status, "not-found");
});

test("does not resolve composer references", () => {
  const draft = file("draft", "chat-a", "m1", "Draft.pdf");
  draft.purpose = "composer";
  const result = resolve("Використай попередній PDF", [message("m1", "chat-a", [draft])]);
  assert.equal(result.status, "not-found");
});

test("reports ambiguity when the latest earlier message has multiple matching files", () => {
  const ambiguous = [message("m1", "chat-a", [
    file("a", "chat-a", "m1", "One.pdf", 0),
    file("b", "chat-a", "m1", "Two.pdf", 1),
  ])];
  const result = resolve("Використай попередній PDF", ambiguous);
  assert.equal(result.status, "ambiguous");
  assert.equal(result.candidates.length, 2);
});

test("resolves all explicitly requested historical PDFs", () => {
  const result = resolve("Використай усі попередні PDF", [
    message("m1", "chat-a", [file("a", "chat-a", "m1", "First.pdf")]),
    message("m2", "chat-a", [file("b", "chat-a", "m2", "Second.pdf")]),
    message("m3", "chat-a", [file("c", "chat-a", "m3", "Current.docx")]),
  ]);
  assert.equal(result.status, "resolved");
  assert.deepEqual(result.references.map((item) => item.attachment.id), ["a", "b"]);
});

test("asks for clarification when several historical file types may be intended", () => {
  const result = resolve("Порівняй із попереднім PDF та DOCX", [
    message("m1", "chat-a", [file("a", "chat-a", "m1", "First.pdf")]),
    message("m2", "chat-a", [file("b", "chat-a", "m2", "Second.docx")]),
  ]);
  assert.equal(result.status, "ambiguous");
  assert.deepEqual(result.candidates.map((item) => item.attachment.id), ["a", "b"]);
});

test("resolves all expressly requested previous file types", () => {
  const result = resolve("Використай обидва попередні файли: PDF та DOCX", [
    message("m1", "chat-a", [file("a", "chat-a", "m1", "First.pdf")]),
    message("m2", "chat-a", [file("b", "chat-a", "m2", "Second.docx")]),
  ]);
  assert.equal(result.status, "resolved");
  assert.deepEqual(result.references.map((item) => item.attachment.id), ["a", "b"]);
});

test("resolves a file from the first previous user message", () => {
  const result = resolve("Використай PDF із першого повідомлення", [
    message("m1", "chat-a", [file("a", "chat-a", "m1", "First.pdf")]),
    message("m2", "chat-a", [file("b", "chat-a", "m2", "Second.pdf")]),
  ]);
  assert.equal(result.status, "resolved");
  assert.equal(result.references[0].attachment.id, "a");
});

test("supports first attachment selection in chronological order", () => {
  const result = resolve("Візьми перший PDF", [
    message("m1", "chat-a", [file("a", "chat-a", "m1", "First.pdf")]),
    message("m2", "chat-a", [file("b", "chat-a", "m2", "Second.pdf")]),
  ]);
  assert.equal(result.status, "resolved");
  assert.equal(result.references[0].attachment.id, "a");
});

test("does not treat ordinary historical conversation as a file reference", () => {
  const result = resolve("Пам'ятаєш, що ми обговорювали раніше?");
  assert.equal(result.status, "none");
});

test("reports a missing explicitly requested file type instead of guessing", () => {
  const result = resolve("Порівняй із попереднім PDF", [
    message("m1", "chat-a", [file("b", "chat-a", "m1", "Report.docx")]),
  ]);
  assert.equal(result.status, "not-found");
});

test("recognizes comprehensive requirement-by-requirement comparison", () => {
  assert.equal(isFullAttachmentComparisonRequest("Перевір кожну вимогу та повноту документа"), true);
  assert.equal(isFullAttachmentComparisonRequest("Compare every requirement against the PDF"), true);
  assert.equal(isFullAttachmentComparisonRequest("Що написано на третій сторінці?"), false);
});
