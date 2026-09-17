//! `--schema` and `--agent-guide`: what the JSON means, from the binary itself.
//!
//! The meanings of this tool's JSON lived in three places -- README prose, `docs/data-model.md`
//! and Rust doc comments -- none of which the CLI could hand to whatever was reading its output.
//! A reader that ran `--json` cold met `"cost_basis": "floor"` with nothing to resolve it against
//! short of the source. Both documents here are compiled in, for the reason the example config
//! is: no binary install channel ships a `docs/` directory.
//!
//! The glossary is data, so it can drift from the code that prints the JSON. `unknown` is what
//! stops that: it walks a real document against the glossary and names every key, every value of
//! a closed vocabulary and every `null` the glossary does not account for. `tests/cli.rs` runs it
//! over each document the binary prints, so a key cannot be added without being described.

use serde_json::Value;

/// Every key and every enum value of every JSON document, as JSON. Printed by `--schema`.
pub const GLOSSARY: &str = include_str!("../docs/json-glossary.json");

/// How to read those documents and what to look for in them. Printed by `--agent-guide`.
pub const AGENT_GUIDE: &str = include_str!("../docs/agent-guide.md");

/// Everything in `document` that the glossary's entry for `flag` does not account for.
///
/// Empty when the document is fully described. Each entry names a path, so a failing test says
/// what to add to `docs/json-glossary.json`.
pub fn unknown(flag: &str, document: &Value) -> Vec<String> {
    let glossary: Value = match serde_json::from_str(GLOSSARY) {
        Ok(glossary) => glossary,
        Err(error) => return vec![format!("docs/json-glossary.json does not parse: {error}")],
    };
    let Some(root) = glossary["documents"].get(flag) else {
        return vec![format!("the glossary has no document {flag}")];
    };
    let mut problems = Vec::new();
    // The document itself is an object whose keys are the entry's `keys`.
    walk(&glossary, root, document, flag, true, &mut problems);
    problems
}

/// `node`'s own keys merged over its shape's, if it names one.
fn keys_of<'a>(glossary: &'a Value, node: &'a Value) -> Vec<(&'a String, &'a Value)> {
    let mut keys = Vec::new();
    if let Some(shape) = node.get("shape").and_then(Value::as_str) {
        if let Some(shared) = glossary["shapes"][shape]["keys"].as_object() {
            keys.extend(shared.iter());
        }
    }
    if let Some(own) = node.get("keys").and_then(Value::as_object) {
        keys.extend(own.iter());
    }
    keys
}

fn walk(
    glossary: &Value,
    node: &Value,
    value: &Value,
    path: &str,
    is_root: bool,
    problems: &mut Vec<String>,
) {
    let declared = node.get("type").and_then(Value::as_str).unwrap_or("object");
    match value {
        Value::Null => {
            if !declared.contains("null") {
                problems.push(format!(
                    "{path} is null, and the glossary types it `{declared}`"
                ));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(
                    glossary,
                    node,
                    item,
                    &format!("{path}[{index}]"),
                    false,
                    problems,
                );
            }
        }
        Value::Object(map) => {
            let keys = keys_of(glossary, node);
            if keys.is_empty() && !is_root {
                problems.push(format!(
                    "{path} is an object the glossary gives no keys for"
                ));
                return;
            }
            for (key, child) in map {
                match keys.iter().find(|(name, _)| *name == key) {
                    Some((_, child_node)) => walk(
                        glossary,
                        child_node,
                        child,
                        &format!("{path}.{key}"),
                        false,
                        problems,
                    ),
                    None => problems.push(format!("{path}.{key} is not in the glossary")),
                }
            }
        }
        Value::String(text) => {
            if let Some(name) = node.get("enum").and_then(Value::as_str) {
                if glossary["enums"][name].get(text).is_none() {
                    problems.push(format!(
                        "{path} is {text:?}, which enum `{name}` does not list"
                    ));
                }
            }
        }
        Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_glossary_parses_and_names_every_document() {
        let glossary: Value = serde_json::from_str(GLOSSARY).expect("the glossary is JSON");
        for flag in [
            "--summary-json",
            "--json",
            "--routing-json",
            "--check-budgets",
        ] {
            assert!(
                glossary["documents"][flag]["keys"].is_object(),
                "no entry for {flag}"
            );
        }
        assert_eq!(
            glossary["schema_version"],
            crate::export::JSON_SCHEMA_VERSION,
            "the glossary describes a different schema version than the exports print"
        );
    }

    /// Every vocabulary the glossary lists is the one the code prints: a label renamed in Rust
    /// would otherwise leave the glossary describing a value nothing emits.
    #[test]
    fn every_enum_lists_exactly_the_labels_the_code_prints() {
        use crate::budget::{AlertLevel, BudgetPeriod};
        use crate::model::{Billing, Category, COST_STATUSES};
        let glossary: Value = serde_json::from_str(GLOSSARY).unwrap();
        let listed = |name: &str| -> Vec<String> {
            let mut keys: Vec<String> = glossary["enums"][name]
                .as_object()
                .unwrap_or_else(|| panic!("no enum {name}"))
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        };
        let sorted = |mut labels: Vec<String>| {
            labels.sort();
            labels
        };
        assert_eq!(
            listed("cost_status"),
            sorted(
                COST_STATUSES
                    .iter()
                    .map(|s| s.label().to_string())
                    .collect()
            )
        );
        assert_eq!(
            listed("category"),
            sorted(
                [
                    Category::Local,
                    Category::Free,
                    Category::Paid,
                    Category::Cloud,
                    Category::Unknown
                ]
                .iter()
                .map(|c| c.label().to_string())
                .collect()
            )
        );
        assert_eq!(
            listed("billing"),
            sorted(
                [Billing::PerToken, Billing::Subscription]
                    .iter()
                    .map(|b| b.label().to_string())
                    .collect()
            )
        );
        assert_eq!(
            listed("budget_level"),
            sorted(
                [
                    AlertLevel::Ok,
                    AlertLevel::Warn,
                    AlertLevel::Critical,
                    AlertLevel::Exceeded
                ]
                .iter()
                .map(|l| l.label().to_string())
                .collect()
            )
        );
        assert_eq!(
            listed("budget_period"),
            sorted(
                [BudgetPeriod::Daily, BudgetPeriod::Monthly]
                    .iter()
                    .map(|p| p.label().to_string())
                    .collect()
            )
        );
        assert_eq!(
            listed("cost_basis"),
            sorted(
                [
                    "exact",
                    "floor",
                    "free",
                    "no_successes",
                    "plus_quota",
                    "quota",
                    "unpriced"
                ]
                .iter()
                .map(|s| s.to_string())
                .collect()
            )
        );
    }

    /// The walker is what makes the glossary trustworthy, so it has to actually object.
    #[test]
    fn the_walker_names_what_the_glossary_does_not_account_for() {
        let fine = json!({"schema_version": 1, "budgets": 0, "alerts": []});
        assert_eq!(unknown("--check-budgets", &fine), Vec::<String>::new());

        let problems = unknown(
            "--check-budgets",
            &json!({
                "schema_version": 1,
                "budgets": null,
                "surprise": true,
                "alerts": [{"scope": "global", "period": "weekly", "level": "OK", "spend": 1.0,
                            "limit": 2.0, "pct": 50.0, "unpriced_requests": 0, "quota_requests": 0}],
            }),
        );
        assert!(
            problems.iter().any(|p| p.contains("surprise")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("budgets is null")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("\"weekly\"")),
            "{problems:?}"
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
    }

    #[test]
    fn the_guide_points_at_the_commands_that_exist() {
        let help = crate::cli::command().render_long_help().to_string();
        for flag in [
            "--summary-json",
            "--schema",
            "--project",
            "--session",
            "--top",
            "--csv",
        ] {
            assert!(
                AGENT_GUIDE.contains(flag),
                "the guide never mentions {flag}"
            );
            assert!(
                help.contains(flag),
                "{flag} is in the guide and not in --help"
            );
        }
    }
}
