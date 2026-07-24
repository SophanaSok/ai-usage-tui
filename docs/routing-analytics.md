# Routing Analytics

## Overview

This document describes the `RoutingEvent` data schema for tracking AI model routing decisions and analytics within the ai-usage-tui project. The schema complements existing usage tracking by capturing the routing decisions made during development tasks, including which agents were used, the models selected, and the outcomes of those routing decisions.

## RoutingEvent struct

```rust
use crate::model::{Category, CostStatus};

#[derive(Clone, Debug, Default)]
pub struct RoutingEvent {
    pub task: String,
    pub phase: String,
    pub agent: String,
    pub model: String,
    pub provider: String,
    pub category: Category,
    pub requests: u64,
    pub tokens: u64,
    pub cost: Option<f64>,
    pub cost_status: CostStatus,
    pub retries: u32,
    pub escalations: u32,
    pub test_result: Option<bool>,
    pub review_defects: u32,
    pub created: i64,
}
```

## routing_event table schema

```sql
CREATE TABLE IF NOT EXISTS routing_event (
    id INTEGER PRIMARY KEY,
    event_id TEXT,
    task TEXT NOT NULL,
    phase TEXT NOT NULL,
    agent TEXT NOT NULL,
    model TEXT NOT NULL,
    provider TEXT NOT NULL,
    category TEXT NOT NULL,
    cost_status TEXT NOT NULL,
    requests INTEGER NOT NULL,
    tokens INTEGER NOT NULL,
    cost REAL,
    retries INTEGER NOT NULL,
    escalations INTEGER NOT NULL,
    test_result INTEGER,
    review_defects INTEGER NOT NULL,
    created INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS routing_event_event_id ON routing_event(event_id);
```

## Example stdin JSON for --record-routing

```json
{
  "agent": "@heavy",
  "model": "glm-5.2:cloud",
  "task": "refactor",
  "phase": "implementation",
  "provider": "ollama",
  "category": "Cloud",
  "requests": 1,
  "tokens": 1250,
  "cost": 0.000375,
  "cost_status": "estimated",
  "retries": 0,
  "escalations": 1,
  "test_result": true,
  "review_defects": 2,
  "created": 1700000000
}
```

## Privacy notes

Collectors may read usage metadata, model identifiers, timestamps, and calculated costs. They must NOT persist or transmit prompts, completions, API keys, or credentials.

The routing event must not contain any prompt content or completion text. All sensitive information is excluded from this schema to maintain the privacy boundary.