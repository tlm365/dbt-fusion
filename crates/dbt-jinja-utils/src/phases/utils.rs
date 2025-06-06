use std::collections::BTreeMap;

use dbt_schemas::schemas::profiles::TargetContext;
use minijinja::value::Value as MinijinjaValue;

pub fn build_target_context_map(
    profile: &str,
    target: &str,
    target_context: TargetContext,
) -> BTreeMap<String, MinijinjaValue> {
    let target_context_val = serde_json::to_value(&target_context).unwrap();
    let mut target_context_map: BTreeMap<String, MinijinjaValue> =
        serde_json::from_value(target_context_val).unwrap();
    target_context_map.insert("profile_name".to_string(), MinijinjaValue::from(profile));
    target_context_map.insert("name".to_string(), MinijinjaValue::from(target));
    target_context_map
}
