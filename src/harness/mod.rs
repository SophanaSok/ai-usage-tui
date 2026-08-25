//! Harnesses: what an agent's own hooks can tell `--record-routing` without guessing.
//!
//! Escalations are derived from usage already collected, but a pass or a fail cannot be inferred
//! from usage metadata and must not be. It has to be *observed* — by whatever ran the agent, at
//! the moment a test command finished. Each module here is one such observer: it reads what the
//! agent hands its hooks, decides whether that was a test run whose outcome the payload actually
//! carries, attributes the attempt to the model that made it, and hands `--record-routing` an
//! event that says only what was seen. Retries, escalations and review defects are never sent:
//! a hook has no way to count them, and an omitted counter is "not reported", which is not `0`.

pub mod claude_code;
pub mod shell;
