import { describe, expect, it } from "vitest";
import { applyTextEdits, positionToStringOffset } from "./documentEdits";
import type { TextEdit } from "./types";

const edit = (startLine: number, startColumn: number, endLine: number, endColumn: number, text: string): TextEdit => ({
  start_line: startLine,
  start_column: startColumn,
  end_line: endLine,
  end_column: endColumn,
  text,
});

describe("applyTextEdits", () => {
  it("returns the original text when there are no edits", () => {
    expect(applyTextEdits("hello", [])).toBe("hello");
  });

  it("applies a single replacement", () => {
    // Replace "world" (cols 7..12 on line 1) with "there".
    expect(applyTextEdits("hello world", [edit(1, 7, 1, 12, "there")])).toBe("hello there");
  });

  it("resolves ALL ranges against the original snapshot, not the mutated text", () => {
    // Regression: the old implementation mutated the buffer after the first edit and
    // resolved the second edit's offsets against that mutated text. Both ranges below
    // are expressed against the ORIGINAL "abcdef"; growing the first must not shift the
    // second. Expected: "abc" -> "XX", "def" -> "YY" => "XXYY".
    const edits = [edit(1, 1, 1, 4, "XX"), edit(1, 4, 1, 7, "YY")];
    expect(applyTextEdits("abcdef", edits)).toBe("XXYY");
  });

  it("is order-independent: edits provided out of order still use original offsets", () => {
    // Same as above but supplied trailing-edit-first. A snapshot-correct implementation
    // sorts by original start offset and applies right-to-left.
    const edits = [edit(1, 4, 1, 7, "YY"), edit(1, 1, 1, 4, "XX")];
    expect(applyTextEdits("abcdef", edits)).toBe("XXYY");
  });

  it("handles multi-line edits where an earlier edit changes line count", () => {
    // Insert a newline inside line 1 AND replace a token on line 2. Old code would have
    // shifted line 2's coordinates by the inserted newline and corrupted the result.
    const text = "foo bar\nbaz qux";
    const edits = [
      edit(1, 4, 1, 4, "\nINSERTED"), // insert at original col 4 of line 1
      edit(2, 1, 2, 4, "ZZZ"), // replace "baz" on the original line 2
    ];
    expect(applyTextEdits(text, edits)).toBe("foo\nINSERTED bar\nZZZ qux");
  });

  it("supports adjacent (touching) edits without treating them as overlapping", () => {
    expect(applyTextEdits("abcd", [edit(1, 1, 1, 3, "X"), edit(1, 3, 1, 5, "Y")])).toBe("XY");
  });

  it("rejects overlapping edits against a single snapshot", () => {
    const edits = [edit(1, 1, 1, 4, "X"), edit(1, 2, 1, 5, "Y")];
    expect(() => applyTextEdits("abcdef", edits)).toThrow(/overlapping/i);
  });

  it("counts astral characters as two UTF-16 units", () => {
    // "😀" is a surrogate pair (width 2). The text after it starts at column 3.
    const text = "😀ab";
    expect(positionToStringOffset(text, 1, 3)).toBe(2);
    expect(applyTextEdits(text, [edit(1, 3, 1, 4, "Z")])).toBe("😀Zb");
  });

  it("throws for positions outside the document", () => {
    expect(() => applyTextEdits("ab", [edit(5, 1, 5, 1, "x")])).toThrow(/outside the document/);
  });
});
