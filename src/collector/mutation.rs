//! Mutation tests over the parsers of other tools' formats.
//!
//! Every fixture under `tests/fixtures/` is a file some other program wrote, and every parser
//! here was written against one version of it. The formats move: a count becomes a string, a key
//! goes, an object turns up where a number was. None of that may take the dashboard down, and
//! none of it may come out the other side as an absurd number.
//!
//! So each source's real fixture is taken apart and put back wrong, one value at a time: every
//! key deleted in turn, and every value replaced in turn with each of [`REPLACEMENTS`]. The
//! damaged records are written into a scratch copy of the source's home, in the source's own
//! layout, and read through the same `load` function the registry gives the dashboard -- so what
//! is exercised is the collector, its cursor and its file handling, not a parser called in a way
//! production never calls it.
//!
//! Deterministic on purpose, and dependency-free. A random fuzzer finds a failure once and then
//! cannot find it again; this one names the record, the path and the replacement, and the same
//! run fails the same way on every machine. The corpus is the fixtures, so a new capture widens
//! it with no change here.
//!
//! **The list of sources is not kept here.** `every_source_of_another_tools_format_survives_mutation`
//! walks `registry::SOURCES`, and a source with no corpus fails it by name.

use std::path::Path;

use serde_json::Value;

use crate::collector::registry::SOURCES;
use crate::collector::SourceRoots;
use crate::model::Usage;

/// What each value is replaced with, in turn. Chosen for the ways a count goes wrong: absent,
/// negative, fractional, too large (as a float, as an `i64::MAX` a SQLite column will hold, and as
/// an integer literal past `u64::MAX`), the wrong
/// type altogether, and a container where a scalar was.
const REPLACEMENTS: &[&str] = &[
    "null",
    "-1",
    "1.5",
    "1e308",
    "9223372036854775807",
    "18446744073709551616",
    "true",
    "\"\"",
    "\"x\"",
    "[]",
    "{}",
];

/// A count above this did not come from a fixture. The largest real one is in the tens of
/// thousands; anything near `u64::MAX` is a float saturating or a negative wrapping.
const ABSURD: u64 = 1 << 53;

/// One damaged copy of a record, and how it was damaged.
struct Variant {
    what: String,
    value: Value,
}

/// Every single-value mutation of `record`: each key deleted, each value replaced.
fn variants(record: &Value, label: &str) -> Vec<Variant> {
    let mut paths = Vec::new();
    collect_paths(record, &mut Vec::new(), &mut paths);
    let mut out = Vec::new();
    for path in paths {
        let shown = path.join(".");
        let mut deleted = record.clone();
        if remove_at(&mut deleted, &path) {
            out.push(Variant {
                what: format!("{label}: `{shown}` deleted"),
                value: deleted,
            });
        }
        for replacement in REPLACEMENTS {
            let mut replaced = record.clone();
            let new: Value = serde_json::from_str(replacement).expect("a JSON literal");
            if let Some(slot) = slot_at(&mut replaced, &path) {
                *slot = new;
                out.push(Variant {
                    what: format!("{label}: `{shown}` = {replacement}"),
                    value: replaced,
                });
            }
        }
    }
    out
}

fn collect_paths(value: &Value, here: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                here.push(key.clone());
                out.push(here.clone());
                collect_paths(child, here, out);
                here.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                here.push(index.to_string());
                out.push(here.clone());
                collect_paths(child, here, out);
                here.pop();
            }
        }
        _ => {}
    }
}

fn slot_at<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut current = value;
    for step in path {
        current = match current {
            Value::Object(map) => map.get_mut(step)?,
            Value::Array(items) => items.get_mut(step.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn remove_at(value: &mut Value, path: &[String]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    match slot_at(value, parents) {
        Some(Value::Object(map)) => map.shift_remove(last).is_some(),
        Some(Value::Array(items)) => match last.parse::<usize>() {
            Ok(index) if index < items.len() => {
                items.remove(index);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every `*.jsonl` line under `dir`, parsed, labelled with its file and line.
fn jsonl_records(dir: &Path) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for path in crate::collector::claude_code::session_files(dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                out.push((format!("{name}:{}", index + 1), value));
            }
        }
    }
    assert!(!out.is_empty(), "no records under {}", dir.display());
    out
}

fn write_jsonl(path: &Path, variants: &[Variant]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut text = String::new();
    for variant in variants {
        text.push_str(&serde_json::to_string(&variant.value).unwrap());
        text.push('\n');
    }
    std::fs::write(path, text).unwrap();
}

/// A source's corpus: the damaged records, and how to lay a set of them out as that source's
/// home so its `load` reads them.
struct Corpus {
    variants: Vec<Variant>,
    plant: fn(&Path, &[Variant]) -> SourceRoots,
}

fn corpus(id: &str) -> Option<Corpus> {
    use crate::collector::{
        claude_code, codex, copilot, gemini, journal, opencode, pricing_refresh,
    };
    let all = |records: Vec<(String, Value)>| -> Vec<Variant> {
        records
            .iter()
            .flat_map(|(label, record)| variants(record, label))
            .collect()
    };
    match id {
        claude_code::ID => Some(Corpus {
            variants: all(jsonl_records(&fixtures().join("claude_capture"))),
            plant: |home, variants| {
                write_jsonl(&home.join("-home-user-project/session.jsonl"), variants);
                let mut roots = SourceRoots::nowhere();
                roots.claude_dir = Some(home.to_path_buf());
                roots
            },
        }),
        codex::ID => {
            let mut records = jsonl_records(&fixtures().join("codex_capture"));
            records.extend(jsonl_records(&fixtures().join("codex_home")));
            Some(Corpus {
                variants: all(records),
                plant: |home, variants| {
                    write_jsonl(
                        &home.join("sessions/2026/09/18/rollout-2026-09-18T10-00-00-0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90.jsonl"),
                        variants,
                    );
                    let mut roots = SourceRoots::nowhere();
                    roots.codex_dir = Some(home.to_path_buf());
                    roots
                },
            })
        }
        gemini::ID => {
            let text = std::fs::read_to_string(fixtures().join("gemini_telemetry.json")).unwrap();
            let records: Vec<(String, Value)> = serde_json::Deserializer::from_str(&text)
                .into_iter::<Value>()
                .map_while(Result::ok)
                .enumerate()
                .map(|(index, value)| (format!("gemini_telemetry.json#{}", index + 1), value))
                .collect();
            assert!(!records.is_empty());
            Some(Corpus {
                variants: all(records),
                plant: |home, variants| {
                    // As the CLI's exporter writes it: pretty-printed objects, back to back.
                    let mut text = String::new();
                    for variant in variants {
                        text.push_str(&serde_json::to_string_pretty(&variant.value).unwrap());
                        text.push('\n');
                    }
                    std::fs::write(home.join("telemetry.json"), text).unwrap();
                    let mut roots = SourceRoots::nowhere();
                    roots.gemini_dir = Some(home.to_path_buf());
                    roots
                },
            })
        }
        opencode::ID => {
            let conn = rusqlite::Connection::open(fixtures().join("opencode_test.db")).unwrap();
            let records: Vec<(String, Value)> = conn
                .prepare("SELECT id, data FROM message")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .flatten()
                .filter_map(|(id, data)| {
                    Some((format!("message {id}"), serde_json::from_str(&data).ok()?))
                })
                .collect();
            assert!(!records.is_empty());
            Some(Corpus {
                variants: all(records),
                plant: |home, variants| {
                    let path = home.join("opencode.db");
                    let mut conn = rusqlite::Connection::open(&path).unwrap();
                    conn.execute_batch(
                        "CREATE TABLE message (id INTEGER PRIMARY KEY, data TEXT NOT NULL, \
                         time_created INTEGER NOT NULL);",
                    )
                    .unwrap();
                    let tx = conn.transaction().unwrap();
                    for (index, variant) in variants.iter().enumerate() {
                        tx.execute(
                            "INSERT INTO message (data, time_created) VALUES (?1, ?2)",
                            rusqlite::params![
                                serde_json::to_string(&variant.value).unwrap(),
                                1_700_000_000_000i64 + index as i64
                            ],
                        )
                        .unwrap();
                    }
                    tx.commit().unwrap();
                    let mut roots = SourceRoots::nowhere();
                    roots.db_path = Some(path);
                    roots
                },
            })
        }
        copilot::ID => Some(Corpus {
            variants: copilot_variants(),
            plant: plant_copilot,
        }),
        // This tool's own file, written only by this tool's own writers, which validate what
        // they take (`--record-event` has its tests in `journal`). Not another tool's format.
        journal::ID => None,
        // Contributes no rows; its input is an HTML page with its own fixtures and tests.
        pricing_refresh::ID => None,
        other => panic!(
            "`{other}` is in the source registry and has no mutation corpus. Add one to \
             `collector::mutation::corpus`: its real fixture, and how to plant damaged copies \
             of its records in a scratch home."
        ),
    }
}

/// Copilot keeps columns, not documents, so a row is turned into an object of its columns, the
/// object is damaged like any other, and `plant_copilot` writes it back as a row. SQLite stores
/// whatever it is handed whatever the column was declared as, which is exactly the hazard.
fn copilot_variants() -> Vec<Variant> {
    let conn =
        rusqlite::Connection::open(fixtures().join("copilot_home/session-store.db")).unwrap();
    let mut statement = conn
        .prepare("SELECT * FROM assistant_usage_events")
        .unwrap();
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(String::from)
        .collect();
    let mut records = Vec::new();
    let mut rows = statement.query([]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let mut object = serde_json::Map::new();
        for (index, column) in columns.iter().enumerate() {
            let value = match row.get_ref(index).unwrap() {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(n) => Value::from(n),
                rusqlite::types::ValueRef::Real(f) => Value::from(f),
                rusqlite::types::ValueRef::Text(t) => {
                    Value::from(String::from_utf8_lossy(t).to_string())
                }
                rusqlite::types::ValueRef::Blob(_) => Value::Null,
            };
            object.insert(column.clone(), value);
        }
        let id = object.get("id").cloned().unwrap_or(Value::Null);
        records.push((
            format!("assistant_usage_events {id}"),
            Value::Object(object),
        ));
    }
    assert!(!records.is_empty());
    records
        .iter()
        .flat_map(|(label, record)| variants(record, label))
        // The primary key is SQLite's, not Copilot's; a damaged one is a constraint failure in
        // this harness and nothing the reader would ever be handed.
        .filter(|variant| !variant.what.contains("`id`"))
        .collect()
}

fn plant_copilot(home: &Path, variants: &[Variant]) -> SourceRoots {
    let path = home.join("session-store.db");
    std::fs::copy(fixtures().join("copilot_home/session-store.db"), &path).unwrap();
    let mut conn = rusqlite::Connection::open(&path).unwrap();
    // The fixture's own rows go, so only damaged ones are read. Constraints off: a deleted
    // `model` is a `NULL` in a `NOT NULL` column, which a real store cannot hold and a future
    // schema might.
    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    let schema: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'assistant_usage_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute_batch("DROP TABLE assistant_usage_events;")
        .unwrap();
    conn.execute_batch(&schema.replace("NOT NULL", "")).unwrap();
    let tx = conn.transaction().unwrap();
    for variant in variants {
        let Value::Object(object) = &variant.value else {
            continue;
        };
        let columns: Vec<&String> = object.keys().filter(|key| *key != "id").collect();
        if columns.is_empty() {
            continue;
        }
        let sql = format!(
            "INSERT INTO assistant_usage_events ({}) VALUES ({})",
            columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(", "),
            vec!["?"; columns.len()].join(", ")
        );
        let values: Vec<rusqlite::types::Value> = columns
            .iter()
            .map(|column| match &object[*column] {
                Value::Null => rusqlite::types::Value::Null,
                Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
                Value::Number(n) => match n.as_i64() {
                    Some(i) => rusqlite::types::Value::Integer(i),
                    None => rusqlite::types::Value::Real(n.as_f64().unwrap_or(f64::MAX)),
                },
                Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                other => rusqlite::types::Value::Text(other.to_string()),
            })
            .collect();
        tx.execute(&sql, rusqlite::params_from_iter(values))
            .unwrap();
    }
    tx.commit().unwrap();
    let mut roots = SourceRoots::nowhere();
    roots.copilot_dir = Some(home.to_path_buf());
    roots
}

/// Read a planted home through the registry's own `load`, then put the rows through what the
/// dashboard and the exports do with them. Returns what is wrong, if anything is.
fn check(id: &str, corpus: &Corpus, variants: &[Variant]) -> Result<(), String> {
    let home = tempfile::tempdir().unwrap();
    let roots = (corpus.plant)(home.path(), variants);
    let spec = SOURCES.iter().find(|spec| spec.id == id).unwrap();
    let outcome = std::panic::catch_unwind(|| {
        let (_, rows) = (spec.load)(&roots).map_err(|error| format!("load failed: {error}"))?;
        // Codex's windows are read from the same rollouts by a second reader.
        if id == crate::collector::codex::ID {
            let _ = crate::limits::load(&roots, 1_789_752_567);
        }
        for row in &rows {
            absurd(row)?;
        }
        // Totals, the summary document and pricing all add the rows up.
        let _: u64 = rows.iter().map(Usage::total_tokens).sum();
        let mut priced = rows;
        crate::pricing::apply_estimated_pricing(
            &mut priced,
            &crate::pricing::PricingEngine::bundled(),
        );
        for row in &priced {
            if row.cost.is_some_and(|cost| !cost.is_finite() || cost < 0.0) {
                return Err(format!("a cost of {:?} for {row:?}", row.cost));
            }
        }
        Ok(())
    });
    match outcome {
        Ok(result) => result,
        Err(_) => Err("panicked".to_string()),
    }
}

fn absurd(row: &Usage) -> Result<(), String> {
    for (name, count) in [
        ("input", row.input),
        ("output", row.output),
        ("reasoning", row.reasoning),
        ("cache_read", row.cache_read),
        ("cache_write", row.cache_write),
    ] {
        if count > ABSURD {
            return Err(format!("{name} = {count}"));
        }
    }
    Ok(())
}

#[test]
fn every_source_of_another_tools_format_survives_mutation() {
    let mut exercised = 0;
    for spec in SOURCES {
        let Some(corpus) = corpus(spec.id) else {
            continue;
        };
        assert!(
            corpus.variants.len() > 100,
            "{}: a corpus of {} mutations is not one",
            spec.id,
            corpus.variants.len()
        );
        exercised += 1;
        // All at once first: thousands of loads would take minutes, and one takes none.
        if check(spec.id, &corpus, &corpus.variants).is_ok() {
            continue;
        }
        // Something in there is wrong. Find it, so the failure names a record and a value.
        let silence = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let culprits: Vec<String> = corpus
            .variants
            .iter()
            .filter_map(|variant| {
                check(spec.id, &corpus, std::slice::from_ref(variant))
                    .err()
                    .map(|problem| format!("  {} -> {problem}", variant.what))
            })
            .take(12)
            .collect();
        std::panic::set_hook(silence);
        panic!(
            "{}: a damaged record broke the collector. First of them:\n{}",
            spec.id,
            culprits.join("\n")
        );
    }
    assert!(exercised >= 5, "only {exercised} sources had a corpus");
}

#[test]
fn the_mutator_deletes_and_replaces_at_every_depth() {
    let record: Value = serde_json::from_str(r#"{"a":{"b":[1,{"c":2}]},"d":3}"#).unwrap();
    let all = variants(&record, "r");
    // Six paths: a, a.b, a.b.0, a.b.1, a.b.1.c, d -- each deleted once and replaced with every
    // replacement.
    assert_eq!(all.len(), 6 * (1 + REPLACEMENTS.len()));
    assert!(
        all.iter()
            .any(|v| v.what == "r: `a.b.1.c` deleted"
                && v.value["a"]["b"][1] == serde_json::json!({}))
    );
    assert!(all
        .iter()
        .any(|v| v.what == "r: `a.b.1.c` = 1e308" && v.value["a"]["b"][1]["c"] == 1e308));
    assert!(all
        .iter()
        .any(|v| v.what == "r: `d` = -1" && v.value["d"] == -1 && v.value["a"] == record["a"]));
}
