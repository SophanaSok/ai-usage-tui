//! Decide whether an agent's usage is billed per token or against a subscription.
//!
//! Claude Code writes identical transcripts whether it runs on an API key or a Pro/Max plan,
//! and Codex does the same for API keys and ChatGPT plans. Nothing on a usage line says which.
//! Priced at list rates, a subscription's traffic reads as real spend and trips budgets on
//! money that was never charged — the exact number this tool exists to refuse to invent.
//!
//! The decision is made once per source, not per row, from signals in this order:
//!
//! 1. An explicit `billing = "subscription" | "api"` under the collector's config table.
//! 2. An API-key environment variable for the agent — a key in the environment means the
//!    agent bills per token, whatever else is true.
//! 3. Claude only: the `oauthAccount` block in Claude Code's own `~/.claude.json`. Only its
//!    presence and its rate-limit-tier string are read; the parsed document is dropped before
//!    this function returns. The file also holds the account's email, name, organisation and
//!    per-project prompt history, none of which is retained or logged. Credentials are never
//!    read: not `.credentials.json`, not `settings.json` (whose `env` block can hold the key
//!    itself), not Codex's `auth.json`.
//! 4. The plan label Omarchy's agents panel already derived, when its record is present.
//! 5. Otherwise per-token, with the reason `"unknown"` so the status line can say so. A
//!    subscription mistaken for API billing shows a visible, false alert; the reverse hides
//!    real spend silently, which is the worse failure.

use std::path::Path;

use serde::Deserialize;

use crate::model::Billing;

/// The user's instruction, from config or `--claude-billing`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BillingSetting {
    #[default]
    Auto,
    Subscription,
    Api,
}

/// The outcome, with enough provenance to print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub billing: Billing,
    /// The plan, when a signal named one — "Max 20x", "Pro".
    pub tier: Option<String>,
    /// Which signal decided. Stable strings, so the status line and the log can name them.
    pub reason: &'static str,
}

impl Decision {
    pub const REASON_UNKNOWN: &'static str = "unknown";

    /// Whether any evidence was found. An unknown decision is re-examined on the next poll; a
    /// decided one is kept, so a half-written config file cannot flip a row's status mid-session.
    pub fn is_evidenced(&self) -> bool {
        self.reason != Self::REASON_UNKNOWN
    }

    /// One phrase for the source line: what was decided and why, never the inputs.
    pub fn describe(&self, config_key: &str) -> String {
        match (self.billing, &self.tier) {
            (Billing::Subscription, Some(tier)) => format!("subscription {tier}"),
            (Billing::Subscription, None) => "subscription".to_string(),
            (Billing::PerToken, _) if self.is_evidenced() => "api billing".to_string(),
            (Billing::PerToken, _) => format!("billing unknown — set [{config_key}] billing"),
        }
    }
}

/// Evidence the detector may consult. Injected so the decision is a pure function of its
/// inputs and a test can supply a planted file and a fake environment.
pub struct Signals<'a> {
    /// Claude Code's `~/.claude.json`, or `None` for agents that have no such file.
    pub claude_json: Option<&'a Path>,
    /// Whether an environment variable is set and non-empty.
    pub env_has: &'a dyn Fn(&str) -> bool,
    /// The `tierLabel` from Omarchy's usage record for this agent, when one exists.
    pub omarchy_tier: Option<&'a str>,
}

/// Carry a decision across polls: evidence, once found, outlives a momentary lack of it.
///
/// Claude Code rewrites its config document constantly, and a poll that catches it half-written
/// must not flip the rows it collects to a different status from the rows already merged. An
/// unknown decision is always replaced, so a later sign-in is picked up.
pub fn resolve_sticky(agent: &str, previous: Option<Decision>, fresh: Decision) -> Decision {
    match previous {
        Some(previous) if previous.is_evidenced() && !fresh.is_evidenced() => previous,
        Some(previous) if previous == fresh => previous,
        _ => {
            crate::logging::info(
                "billing",
                &format!(
                    "{agent}: {} ({})",
                    fresh.describe(&format!("collectors.{agent}")),
                    fresh.reason
                ),
            );
            fresh
        }
    }
}

/// Whether an environment variable is set and non-empty. The production `Signals::env_has`.
pub fn env_has(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// Environment variables whose presence means the agent bills per token.
pub fn api_env_vars(agent: &str) -> &'static [&'static str] {
    match agent {
        "claude_code" => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
        ],
        "codex" => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        // Gemini CLI bills per token on an API key and against a plan on OAuth. These are the
        // variables it checks itself before falling back to a browser sign-in.
        "gemini" => &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_GENAI_USE_VERTEXAI",
        ],
        _ => &[],
    }
}

pub fn detect(agent: &str, setting: BillingSetting, signals: &Signals<'_>) -> Decision {
    match setting {
        BillingSetting::Subscription => {
            return Decision {
                billing: Billing::Subscription,
                tier: signals.omarchy_tier.map(str::to_string),
                reason: "config",
            }
        }
        BillingSetting::Api => {
            return Decision {
                billing: Billing::PerToken,
                tier: None,
                reason: "config",
            }
        }
        BillingSetting::Auto => {}
    }

    if api_env_vars(agent)
        .iter()
        .any(|name| (signals.env_has)(name))
    {
        return Decision {
            billing: Billing::PerToken,
            tier: None,
            reason: "api key in environment",
        };
    }

    if let Some(path) = signals.claude_json {
        if let Some(account) = read_oauth_account(path) {
            return Decision {
                billing: Billing::Subscription,
                tier: account.tier_label(),
                reason: "claude.json oauthAccount",
            };
        }
    }

    if let Some(tier) = signals.omarchy_tier.filter(|tier| !tier.trim().is_empty()) {
        return Decision {
            billing: Billing::Subscription,
            tier: Some(tier.trim().to_string()),
            reason: "omarchy record",
        };
    }

    Decision {
        billing: Billing::PerToken,
        tier: None,
        reason: Decision::REASON_UNKNOWN,
    }
}

/// The only two keys read from `~/.claude.json`, and only under `oauthAccount`.
///
/// Deliberately not a struct of the whole file: `serde` would then have to see every field,
/// and a `Debug` of the result would carry the account's name and email into a log line.
#[derive(Debug, Default, Deserialize)]
struct OauthAccount {
    #[serde(rename = "organizationRateLimitTier")]
    organization_rate_limit_tier: Option<String>,
    #[serde(rename = "userRateLimitTier")]
    user_rate_limit_tier: Option<String>,
}

impl OauthAccount {
    fn tier_label(&self) -> Option<String> {
        self.user_rate_limit_tier
            .as_deref()
            .and_then(plan_label)
            .or_else(|| {
                self.organization_rate_limit_tier
                    .as_deref()
                    .and_then(plan_label)
            })
    }
}

/// `Some` when the file has an `oauthAccount` object, whatever it contains; `None` when the
/// file is absent, unreadable, not JSON, or has no such block. Every failure reads as "no
/// evidence", never as an error: the file is rewritten by Claude Code constantly and a
/// half-written snapshot must not stop a poll.
fn read_oauth_account(path: &Path) -> Option<OauthAccount> {
    let text = std::fs::read_to_string(path).ok()?;
    let document: serde_json::Value = serde_json::from_str(&text).ok()?;
    let account = document.get("oauthAccount")?;
    if !account.is_object() {
        return None;
    }
    Some(serde_json::from_value(account.clone()).unwrap_or_default())
}

/// A display label from a rate-limit tier id such as `default_claude_max_20x`.
///
/// The same reading Omarchy's collector applies to the tier from the credential store, so the
/// two tools name a plan identically. A tier that names no known plan yields `None` — the
/// decision is still "subscription", just without a label.
pub fn plan_label(tier: &str) -> Option<String> {
    let lower = tier.to_ascii_lowercase();
    if let Some(rest) = lower.split("max_").nth(1) {
        let multiplier: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == 'x')
            .collect();
        if multiplier.ends_with('x') && multiplier.len() > 1 {
            return Some(format!("Max {multiplier}"));
        }
        return Some("Max".to_string());
    }
    for (needle, label) in [
        ("enterprise", "Enterprise"),
        ("team", "Team"),
        ("pro", "Pro"),
    ] {
        if lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token == needle)
        {
            return Some(label.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> bool {
        false
    }

    fn planted_claude_json(dir: &Path) -> std::path::PathBuf {
        let path = dir.join(".claude.json");
        std::fs::write(
            &path,
            r#"{
              "numStartups": 12,
              "oauthAccount": {
                "accountUuid": "00000000-0000-0000-0000-000000000000",
                "emailAddress": "planted@example.invalid",
                "fullName": "Planted Person",
                "organizationName": "Planted Org",
                "organizationRateLimitTier": "default_claude_max_5x",
                "userRateLimitTier": null,
                "billingType": "stripe_subscription"
              },
              "projects": {
                "/home/planted/repo": {
                  "history": [{"display": "AWS_SECRET_ACCESS_KEY=hunter2"}]
                }
              }
            }"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn a_planted_claude_json_yields_subscription_and_nothing_else() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = planted_claude_json(dir.path());
        let signals = Signals {
            claude_json: Some(&path),
            env_has: &no_env,
            omarchy_tier: None,
        };
        let decision = detect("claude_code", BillingSetting::Auto, &signals);
        assert_eq!(decision.billing, Billing::Subscription);
        assert_eq!(decision.tier.as_deref(), Some("Max 5x"));

        // Nothing but the tier may leave the file: not the email, the name, the org, nor the
        // prompt history that shares the document.
        let rendered = format!(
            "{:?} {}",
            decision,
            decision.describe("collectors.claude_code")
        );
        for planted in [
            "planted@example.invalid",
            "Planted",
            "hunter2",
            "AWS_SECRET",
        ] {
            assert!(!rendered.contains(planted), "{planted} leaked: {rendered}");
        }
    }

    #[test]
    fn an_api_key_in_the_environment_beats_the_oauth_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = planted_claude_json(dir.path());
        let env_has = |name: &str| name == "ANTHROPIC_API_KEY";
        let signals = Signals {
            claude_json: Some(&path),
            env_has: &env_has,
            omarchy_tier: None,
        };
        let decision = detect("claude_code", BillingSetting::Auto, &signals);
        assert_eq!(decision.billing, Billing::PerToken);
        assert_eq!(decision.reason, "api key in environment");
    }

    #[test]
    fn an_explicit_setting_beats_every_signal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = planted_claude_json(dir.path());
        let env_has = |name: &str| name == "ANTHROPIC_API_KEY";
        let signals = Signals {
            claude_json: Some(&path),
            env_has: &env_has,
            omarchy_tier: Some("Max 20x"),
        };
        assert_eq!(
            detect("claude_code", BillingSetting::Api, &signals).billing,
            Billing::PerToken
        );
        let forced = detect("claude_code", BillingSetting::Subscription, &signals);
        assert_eq!(forced.billing, Billing::Subscription);
        assert_eq!(forced.reason, "config");
        // With no files at all, the setting alone still decides.
        let bare = Signals {
            claude_json: None,
            env_has: &no_env,
            omarchy_tier: None,
        };
        assert_eq!(
            detect("claude_code", BillingSetting::Subscription, &bare).billing,
            Billing::Subscription
        );
    }

    #[test]
    fn a_missing_or_truncated_file_is_no_evidence_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("absent.json");
        let signals = Signals {
            claude_json: Some(&missing),
            env_has: &no_env,
            omarchy_tier: None,
        };
        let decision = detect("claude_code", BillingSetting::Auto, &signals);
        assert_eq!(decision.billing, Billing::PerToken);
        assert_eq!(decision.reason, Decision::REASON_UNKNOWN);
        assert!(!decision.is_evidenced());
        assert!(decision
            .describe("collectors.claude_code")
            .contains("billing unknown"));

        let truncated = dir.path().join(".claude.json");
        std::fs::write(&truncated, "{\"oauthAccount\": {\"organizationRateLimitTi").unwrap();
        let signals = Signals {
            claude_json: Some(&truncated),
            env_has: &no_env,
            omarchy_tier: None,
        };
        assert_eq!(
            detect("claude_code", BillingSetting::Auto, &signals).reason,
            Decision::REASON_UNKNOWN
        );
    }

    #[test]
    fn an_omarchy_tier_label_is_evidence_but_an_empty_one_is_not() {
        let signals = Signals {
            claude_json: None,
            env_has: &no_env,
            omarchy_tier: Some("Max 20x"),
        };
        let decision = detect("claude_code", BillingSetting::Auto, &signals);
        assert_eq!(decision.billing, Billing::Subscription);
        assert_eq!(decision.tier.as_deref(), Some("Max 20x"));
        assert_eq!(decision.reason, "omarchy record");

        let blank = Signals {
            claude_json: None,
            env_has: &no_env,
            omarchy_tier: Some("  "),
        };
        assert_eq!(
            detect("codex", BillingSetting::Auto, &blank).reason,
            Decision::REASON_UNKNOWN
        );
    }

    #[test]
    fn codex_has_its_own_api_variables_and_no_oauth_file() {
        let env_has = |name: &str| name == "OPENAI_API_KEY";
        let signals = Signals {
            claude_json: None,
            env_has: &env_has,
            omarchy_tier: Some("Plus"),
        };
        assert_eq!(
            detect("codex", BillingSetting::Auto, &signals).billing,
            Billing::PerToken
        );
        // An Anthropic key says nothing about Codex.
        let env_has = |name: &str| name == "ANTHROPIC_API_KEY";
        let signals = Signals {
            claude_json: None,
            env_has: &env_has,
            omarchy_tier: Some("Plus"),
        };
        assert_eq!(
            detect("codex", BillingSetting::Auto, &signals).billing,
            Billing::Subscription
        );
    }

    #[test]
    fn plan_labels_follow_omarchy_naming() {
        assert_eq!(
            plan_label("default_claude_max_20x").as_deref(),
            Some("Max 20x")
        );
        assert_eq!(
            plan_label("default_claude_max_5x").as_deref(),
            Some("Max 5x")
        );
        assert_eq!(plan_label("default_claude_pro").as_deref(), Some("Pro"));
        assert_eq!(plan_label("enterprise_seat").as_deref(), Some("Enterprise"));
        assert_eq!(plan_label("something_else"), None);
        assert_eq!(plan_label(""), None);
    }

    #[test]
    fn a_user_tier_wins_over_the_organisation_tier() {
        let account = OauthAccount {
            organization_rate_limit_tier: Some("default_claude_max_20x".into()),
            user_rate_limit_tier: Some("default_claude_pro".into()),
        };
        assert_eq!(account.tier_label().as_deref(), Some("Pro"));
    }
}
