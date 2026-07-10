import { describe, expect, it } from "vitest";
import { parseHtmlArtifactLang } from "../../components/Aspector/AspectorChatMessages";

describe("parseHtmlArtifactLang", () => {
  it("auto-previews html with an explicit preview/live/run modifier", () => {
    expect(parseHtmlArtifactLang("html preview")).toEqual({ autoPreview: true });
    expect(parseHtmlArtifactLang("html live")).toEqual({ autoPreview: true });
    expect(parseHtmlArtifactLang("html run")).toEqual({ autoPreview: true });
    expect(parseHtmlArtifactLang("HTML Preview")).toEqual({ autoPreview: true });
    expect(parseHtmlArtifactLang("htm,live")).toEqual({ autoPreview: true });
  });

  it("treats a bare html fence as a code-first artifact", () => {
    expect(parseHtmlArtifactLang("html")).toEqual({ autoPreview: false });
    expect(parseHtmlArtifactLang("htm")).toEqual({ autoPreview: false });
  });

  it("returns null for non-html languages and empty info-strings", () => {
    expect(parseHtmlArtifactLang(undefined)).toBeNull();
    expect(parseHtmlArtifactLang("")).toBeNull();
    expect(parseHtmlArtifactLang("ts")).toBeNull();
    expect(parseHtmlArtifactLang("xml")).toBeNull();
    expect(parseHtmlArtifactLang("htmlbars")).toBeNull();
  });
});
