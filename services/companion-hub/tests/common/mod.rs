//! Shared helpers: validate JSON against the schemas exported from packages/capture-contracts.
use serde_json::Value;
use std::path::PathBuf;

pub fn schema(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../packages/capture-contracts/schema/{name}.schema.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p:?}: {e}"))).unwrap()
}

pub fn assert_valid(name: &str, value: &Value) {
    let s = schema(name);
    let validator = jsonschema::validator_for(&s).expect("schema compiles");
    let errors: Vec<String> = validator.iter_errors(value).map(|e| format!("{} at {}", e, e.instance_path())).collect();
    assert!(errors.is_empty(), "{name} invalid: {errors:#?}\nvalue: {}", serde_json::to_string_pretty(value).unwrap());
}
