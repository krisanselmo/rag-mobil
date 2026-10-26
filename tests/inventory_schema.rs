//! Keeps `schema/inventory-v1.schema.json` in sync with `SetInventory`
//! without a JSON Schema validator: same properties, same required fields,
//! compatible types.

use std::collections::BTreeSet;

use rag_mobil::{Part, SetInventory, SetNumber};
use serde_json::{Map, Value};

const SCHEMA: &str = include_str!("../schema/inventory-v1.schema.json");

fn schema() -> Value {
    serde_json::from_str(SCHEMA).expect("the schema is valid JSON")
}

/// Every optional field set, so that it serializes to a non-null value.
fn full_inventory() -> SetInventory {
    SetInventory {
        set_number: SetNumber::parse("3666").unwrap(),
        name: "Kings Large Castle".into(),
        theme: Some("Knights".into()),
        focus: Some("Castle".into()),
        release_date: Some("1993/09".into()),
        parts: vec![Part {
            number: "30 00 4950".into(),
            name: "Castle Trumpeter".into(),
            color: Some("red".into()),
            quantity: Some(1),
            category: "Klicky".into(),
            components: vec!["Wig (hair), classic male - brown".into()],
            image: Some("https://playmodb.org/parts/0/00/30%2000%20495.gif".into()),
            nickname: Some("Tutu".into()),
        }],
        source_url: "https://playmodb.org/cgi-bin/showinv.pl?setnum=3666".into(),
    }
}

fn properties(object_schema: &Value) -> BTreeSet<String> {
    object_schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect()
}

fn required(object_schema: &Value) -> BTreeSet<String> {
    object_schema["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn types(property_schema: &Value) -> Vec<&str> {
    match &property_schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().map(|t| t.as_str().unwrap()).collect(),
        other => panic!("unexpected type {other}"),
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_u64() || n.is_i64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The serialized object has exactly the schema's properties, each with an
/// allowed type, and optional (nullable) fields are exactly the non-required ones.
fn check_object(what: &str, object_schema: &Value, object: &Map<String, Value>) {
    let keys: BTreeSet<String> = object.keys().cloned().collect();
    assert_eq!(keys, properties(object_schema), "{what}: properties");
    assert!(required(object_schema).is_subset(&keys), "{what}: required");
    for (key, value) in object {
        let property = &object_schema["properties"][key];
        let allowed = types(property);
        assert!(
            allowed.contains(&json_type(value)),
            "{what}.{key}: {value} is not {allowed:?}"
        );
        let nullable = allowed.contains(&"null");
        assert_eq!(
            nullable,
            !required(object_schema).contains(key),
            "{what}.{key}: nullable iff not required"
        );
    }
}

fn keep_only(object: &mut Map<String, Value>, keys: &BTreeSet<String>) {
    object.retain(|key, _| keys.contains(key));
}

#[test]
fn schema_is_draft_2020_12() {
    let schema = schema();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        schema["properties"]["parts"]["items"]["$ref"],
        "#/$defs/part"
    );
    assert!(
        schema["$id"]
            .as_str()
            .unwrap()
            .ends_with("inventory-v1.schema.json")
    );
}

#[test]
fn schema_properties_match_the_serialized_inventory() {
    let schema = schema();
    let value = serde_json::to_value(full_inventory()).unwrap();
    check_object("inventory", &schema, value.as_object().unwrap());
    for part in value["parts"].as_array().unwrap() {
        check_object("part", &schema["$defs"]["part"], part.as_object().unwrap());
    }
}

#[test]
fn required_fields_are_exactly_the_non_optional_ones() {
    let schema = schema();
    let root_required = required(&schema);
    let part_required = required(&schema["$defs"]["part"]);

    // Only the required fields: deserializes, the rest defaults to None.
    let mut minimal = serde_json::to_value(full_inventory()).unwrap();
    let root = minimal.as_object_mut().unwrap();
    keep_only(root, &root_required);
    for part in root["parts"].as_array_mut().unwrap() {
        keep_only(part.as_object_mut().unwrap(), &part_required);
    }
    let parsed: SetInventory = serde_json::from_value(minimal.clone()).expect("minimal");
    assert_eq!(parsed.theme, None);
    assert_eq!(parsed.parts[0].quantity, None);

    // Without any one of them: rejected.
    for key in &root_required {
        let mut doc = minimal.clone();
        doc.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<SetInventory>(doc).is_err(),
            "inventory.{key} should be required by serde"
        );
    }
    for key in &part_required {
        let mut doc = minimal.clone();
        doc["parts"][0].as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<SetInventory>(doc).is_err(),
            "part.{key} should be required by serde"
        );
    }
}

#[test]
fn saved_inventories_follow_the_schema() {
    let schema = schema();
    let root_properties = properties(&schema);
    let part_properties = properties(&schema["$defs"]["part"]);
    let part_required = required(&schema["$defs"]["part"]);
    for json in [
        include_str!("e2e/3799/inventory.json"),
        include_str!("e2e/3799+70145/inventory.json"),
    ] {
        let value: Value = serde_json::from_str(json).unwrap();
        let inventories = match value {
            Value::Array(many) => many,
            one => vec![one],
        };
        for inventory in inventories {
            let object = inventory.as_object().unwrap();
            let keys: BTreeSet<String> = object.keys().cloned().collect();
            assert_eq!(keys, root_properties);
            for part in object["parts"].as_array().unwrap() {
                // Optional fields may be absent (`image`, from before it existed).
                let keys: BTreeSet<String> = part.as_object().unwrap().keys().cloned().collect();
                assert!(keys.is_subset(&part_properties), "{keys:?}");
                assert!(part_required.is_subset(&keys), "{keys:?}");
            }
            serde_json::from_value::<SetInventory>(inventory).unwrap();
        }
    }
}
