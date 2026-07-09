import { afterEach, describe, expect, it } from "vitest";
import type { AiChatMessage } from "./../../chat/types";
import {
  disposeGoalRun,
  evaluateGoalCondition,
  getActiveGoalRun,
  getGoalRunSnapshot,
  normalizeAnswerForRepeatGuard,
  repeatedAnswerSimilarity,
  startGoalRun,
} from "./session-goal-run";

const SESSION = "refusal-guard-test";

const assistantTurn = (id: string, content: string, toolCalls = 0): AiChatMessage => ({
  id,
  role: "assistant",
  content,
  timestamp: Date.now(),
  toolCalls: Array.from({ length: toolCalls }, (_, index) => ({
    id: `${id}-tool-${index}`,
    tool: "Read",
    status: "success" as const,
    startTime: 0,
  })),
});

// Three rephrasings of the same refusal — the shape of the real bug: the model
// declines, Automatic re-prompts, the model declines again with new wording.
const REFUSAL_1 =
  "Читы для Brawl Stars я не сделаю — это вредоносный код для онлайн-игры, вредит другим игрокам и банит аккаунт. Легальные альтернативы: симулятор гача-боксов, трекер прокачки бойцов, лендинг клана.";
const REFUSAL_2 =
  "Читы для Brawl Stars не сделаю — вредоносный код для онлайн-игры вредит другим игрокам и ведёт к бану аккаунта. Могу собрать легальные альтернативы: симулятор гача-боксов, трекер прокачки бойцов или лендинг клана.";
const REFUSAL_3 =
  "Нет, читы для Brawl Stars я не сделаю: это вредоносный код, он вредит другим игрокам онлайн-игры и банит аккаунт. Выбери легальную альтернативу — симулятор гача-боксов, трекер прокачки или лендинг клана.";

function startRun() {
  const run = startGoalRun(SESSION, "Сделай читы для Brawl Stars", {
    agentMode: "automatic",
    toolRoundLimit: "unlimited" as never,
  });
  expect(run).not.toBeNull();
}

afterEach(() => disposeGoalRun(SESSION));

describe("goal-run refusal guard", () => {
  it("stops an Automatic run after three near-identical no-tool answers", () => {
    startRun();
    const first = evaluateGoalCondition(SESSION, [assistantTurn("a1", REFUSAL_1)], "automatic");
    expect(first.status).toBe("continue");
    const second = evaluateGoalCondition(SESSION, [assistantTurn("a2", REFUSAL_2)], "automatic");
    expect(second.status).toBe("continue");
    const third = evaluateGoalCondition(SESSION, [assistantTurn("a3", REFUSAL_3)], "automatic");
    expect(third.status).toBe("blocked");
    expect(getActiveGoalRun(SESSION)).toBeNull();
    expect(getGoalRunSnapshot(SESSION)?.phase).toBe("blocked");
  });

  it("stops after two repeats when the refusal carries [goal:blocked]", () => {
    startRun();
    const first = evaluateGoalCondition(
      SESSION,
      [assistantTurn("a1", `${REFUSAL_1}\n[goal:blocked]`)],
      "automatic",
    );
    // First blocked marker in Automatic converts to a self-decide continue.
    expect(first.status).toBe("continue");
    const second = evaluateGoalCondition(
      SESSION,
      [assistantTurn("a2", `${REFUSAL_2}\n[goal:blocked]`)],
      "automatic",
    );
    expect(second.status).toBe("blocked");
  });

  it("does not trip on distinct answers or on turns with tool work", () => {
    startRun();
    evaluateGoalCondition(SESSION, [assistantTurn("a1", REFUSAL_1)], "automatic");
    // Real tool work resets the guard...
    evaluateGoalCondition(SESSION, [assistantTurn("a2", "Читаю структуру проекта и план работ.", 2)], "automatic");
    // ...so two more similar answers are again below the threshold.
    evaluateGoalCondition(SESSION, [assistantTurn("a3", REFUSAL_2)], "automatic");
    const result = evaluateGoalCondition(
      SESSION,
      [assistantTurn("a4", "Совсем другой ответ: собрал лендинг клана, деплой на pages готов, тесты зелёные.")],
      "automatic",
    );
    expect(result.status).toBe("continue");
    expect(getActiveGoalRun(SESSION)).not.toBeNull();
  });
});

describe("repeatedAnswerSimilarity", () => {
  it("scores rephrased refusals as similar and unrelated answers as distinct", () => {
    const a = normalizeAnswerForRepeatGuard(REFUSAL_1);
    const b = normalizeAnswerForRepeatGuard(REFUSAL_2);
    const c = normalizeAnswerForRepeatGuard("Готово: собрал симулятор гача-боксов, добавил анимацию открытия и задеплоил.");
    expect(repeatedAnswerSimilarity(a, b)).toBeGreaterThanOrEqual(0.7);
    expect(repeatedAnswerSimilarity(a, c)).toBeLessThan(0.7);
  });

  it("strips goal markers before comparing", () => {
    const withMarker = normalizeAnswerForRepeatGuard(`${REFUSAL_1}\n[goal:blocked]`);
    const without = normalizeAnswerForRepeatGuard(REFUSAL_1);
    expect(repeatedAnswerSimilarity(withMarker, without)).toBe(1);
  });
});
