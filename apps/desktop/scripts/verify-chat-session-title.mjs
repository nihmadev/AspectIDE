const titleModelPattern = /haiku|mini|nano|flash|small|fast|lite|8b/i;

function normalizeChatSessionTitle(value) {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (!normalized) return "New chat";
  const stripped = normalized.replace(/^["'`]+|["'`]+$/g, "").replace(/[.!?]+$/g, "").trim();
  if (!stripped) return "New chat";
  return stripped.length > 42 ? `${stripped.slice(0, 42).trimEnd()}...` : stripped;
}

function resolveTitleGenerationModel(provider, activeModel) {
  const ranked = provider.models
    .map((model) => {
      const haystack = `${model.id} ${model.alias} ${model.name}`.toLowerCase();
      let score = 0;
      if (titleModelPattern.test(haystack)) score += 40;
      if (model.id === activeModel.id) score += 5;
      return { model, score };
    })
    .filter((entry) => entry.score > 0)
    .sort((left, right) => right.score - left.score);
  return ranked[0]?.model ?? activeModel;
}

const provider = {
  models: [
    { id: "gpt-5", name: "GPT-5", alias: "gpt-5" },
    { id: "gpt-5-mini", name: "GPT-5 Mini", alias: "gpt-5-mini" },
  ],
};
const active = provider.models[0];
const picked = resolveTitleGenerationModel(provider, active);
if (picked.id !== "gpt-5-mini") {
  console.error("expected mini model for titles, got", picked.id);
  process.exit(1);
}
if (normalizeChatSessionTitle('  "Fix PDF preview"  ') !== "Fix PDF preview") {
  console.error("normalize failed");
  process.exit(1);
}
console.log("chat session title verification passed");