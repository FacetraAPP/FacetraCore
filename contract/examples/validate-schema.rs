//! Test helper for checking actual CLI schema output against actual YAML output.
use std::io::{self, Read};
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let request: serde_json::Value = serde_json::from_str(&input).unwrap();
    let schema = &request["schema"];
    let value = facetra_contract::parse(request["yaml"].as_str().unwrap()).unwrap();
    jsonschema::validator_for(schema)
        .unwrap()
        .validate(&value)
        .unwrap();
}
