use super::schema::{tool, opt, req_arr_items, opt_arr_items, opt_str_arr, steps_item_schema, alternatives_item_schema};

pub fn register(tools: &mut Vec<serde_json::Value>) {
    let steps_schema = steps_item_schema();
    let alt_schema = alternatives_item_schema();
    tools.push(tool(
        "PresentPlan",
        "Present a structured, reviewable execution plan to the user. Renders an expandable plan card and pins the plan as the session goal + task list. In Plan/Agent mode the user presses Start to hand it to Agent execution (do not edit before that). In Automatic mode execution auto-starts. Scale the plan to the task's complexity and risk \u{2014} it is NOT a flat list of phases. A strong plan covers five reasoning phases (a deterministic quality gate scores them and coaches whatever is missing): (1) DECOMPOSE into concrete file-level `steps` (each = a specific action on a named file/module with its acceptance check, never vague labels like 'implement business logic'); (2) ALTERNATIVES \u{2014} in `alternatives`, name the key decision(s): the approach you chose and why it wins over the option you rejected (the tradeoff); (3) CRITIQUE \u{2014} in `risks`, the failure modes and hidden assumptions of the riskiest step (what breaks, under what input/timing); (4) SYNTHESIS \u{2014} the chosen path's rationale in `summary`; (5) VERIFY \u{2014} in `verification`, the tests/build/checks that prove it works, plus a rollback/recovery trigger for risky changes. Riskier work (auth, payments, migrations, concurrency, data-loss, public APIs) earns more steps, an explicit decision, named risks, and verification; trivial work stays terse (steps alone are fine). Prefer this over a plain prose checklist for multi-step work.",
        &[
            req_arr_items("steps", "Ordered steps: strings or { title, detail, file } objects.", steps_schema),
            opt("title", "string", "Short plan title."),
            opt("summary", "string", "One-paragraph summary of the goal/approach + why this path (synthesis)."),
            opt_arr_items("alternatives", "Key decisions: strings or { option, tradeoff } objects \u{2014} the approach chosen and why it beats the rejected one.", alt_schema),
            opt_str_arr("risks", "Failure modes / hidden assumptions of the riskiest steps (strings)."),
            opt_str_arr("verification", "Checks that prove it works + rollback trigger (strings)."),
        ],
    ));
}
