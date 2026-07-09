use crate::types::{PlanDecision, PlanStep};

const PLAN_RISK_MARKERS: &[&str] = &[
    "security", "secure", "auth", "password", "token", "payment", "billing",
    "concurren", "migrat", "schema", "distributed", "performance", "rollback",
    "delete", "destructive", "public api", "breaking",
];

const PLAN_VAGUE_LABELS: &[&str] = &[
    "set up the project", "set up project", "setup",
    "implement business logic", "implement logic", "implement the feature",
    "add documentation", "write docs", "do the rest", "finish up",
    "wire everything", "make it work", "clean up", "testing", "polish",
];

pub fn assess_plan_quality(
    title: &str,
    summary: &str,
    steps: &[PlanStep],
    alternatives: &[PlanDecision],
    risks: &[String],
    verification: &[String],
) -> (f64, Vec<String>) {
    let mut coaching: Vec<String> = Vec::new();
    let haystack = {
        let mut s = format!("{title}\n{summary}");
        for step in steps {
            s.push('\n');
            s.push_str(&step.title);
            s.push('\n');
            s.push_str(&step.detail);
        }
        for alt in alternatives {
            s.push('\n');
            s.push_str(&alt.option);
            s.push('\n');
            s.push_str(&alt.tradeoff);
        }
        for risk in risks {
            s.push('\n');
            s.push_str(risk);
        }
        for check in verification {
            s.push('\n');
            s.push_str(check);
        }
        s.to_lowercase()
    };

    let risk_hits = PLAN_RISK_MARKERS
        .iter()
        .filter(|m| haystack.contains(**m))
        .count();
    let required_steps = (3 + risk_hits).min(8);
    let expects_alternatives = risk_hits >= 1 || steps.len() >= 5;
    let expects_critique = risk_hits >= 1 || steps.len() >= 4;

    let mut score = 1.0_f64;

    if steps.len() < required_steps {
        score -= 0.2;
        coaching.push(format!(
            "Decompose further — {} step(s) for {}-risk work; aim for ~{}, each a concrete action on a named file/module.",
            steps.len(),
            if risk_hits > 0 { "higher" } else { "this" },
            required_steps
        ));
    }

    let vague = steps
        .iter()
        .filter(|s| {
            let t = s.title.to_lowercase();
            PLAN_VAGUE_LABELS
                .iter()
                .any(|v| t == *v || t.starts_with(v))
        })
        .count();
    let with_anchor = steps
        .iter()
        .filter(|s| !s.file.is_empty() || s.detail.chars().count() >= 24)
        .count();

    if vague > 0 {
        score -= 0.15;
        coaching.push(format!(
            "Replace {vague} vague step label(s) (e.g. 'implement logic', 'add documentation') with a specific action + its acceptance check."
        ));
    }
    if steps.len() >= 3 && with_anchor * 2 < steps.len() {
        score -= 0.1;
        coaching.push(
            "Most steps lack a file or concrete detail — name the file/module each step touches and what proves it done.".to_string(),
        );
    }

    let has_decision = alternatives.iter().any(|a| !a.option.trim().is_empty())
        || [
            "instead of", "rather than", "trade-off", "tradeoff",
            "alternative", " vs ", "chose ", "chosen ", "decided ",
        ]
        .iter()
        .any(|kw| haystack.contains(kw));

    if expects_alternatives && !has_decision {
        score -= 0.2;
        coaching.push(
            "Name the key decision: the approach you chose and why it wins over the alternative(s) (the tradeoff). A plan that weighs options beats one that charges ahead with its first idea.".to_string(),
        );
    }

    let has_critique = risks.iter().any(|r| !r.trim().is_empty())
        || [
            "risk", "fail", "assumption", "assume", "edge case", "race",
            "breaks if", "could break", "fallback",
        ]
        .iter()
        .any(|kw| haystack.contains(kw));

    if expects_critique && !has_critique {
        score -= 0.2;
        coaching.push(
            "Critique the riskiest step: list its failure modes and hidden assumptions — what breaks, under what input/timing, and how you'd catch it.".to_string(),
        );
    }

    let has_verification = verification.iter().any(|v| !v.trim().is_empty())
        || steps.iter().any(|s| {
            let t = format!("{} {}", s.title, s.detail).to_lowercase();
            ["test", "verif", "build", "typecheck", "lint", "run ", "check", "assert", "validate"]
                .iter()
                .any(|kw| t.contains(kw))
        });

    if !has_verification {
        score -= 0.25;
        coaching.push(
            "Add explicit verification: the tests/build/checks that prove it works (plus a rollback trigger for risky changes).".to_string(),
        );
    }

    if risk_hits >= 2 {
        let has_rollback = haystack.contains("rollback")
            || haystack.contains("revert")
            || haystack.contains("checkpoint")
            || haystack.contains("backup");
        if !has_rollback {
            score -= 0.1;
            coaching.push(
                "High-risk plan: name a rollback/recovery path (Checkpoint, revert, or backup) for the riskiest step.".to_string(),
            );
        }
    }

    (score.clamp(0.0, 1.0), coaching)
}
