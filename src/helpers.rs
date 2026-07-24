use serde_json::Value;

pub fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(String::from))
}
pub fn number(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(Value::as_u64).or_else(|| {
                value
                    .get(*key)
                    .and_then(Value::as_f64)
                    .map(|value| value as u64)
            })
        })
        .unwrap_or(0)
}
