use serde_json::{Map, Value};

pub fn json_diff(a: &Value, b: &Value) -> Option<Value> {
    if a == b {
        return None;
    }

    match (a, b) {
        (Value::Object(a_obj), Value::Object(b_obj)) => {
            let mut diff = Map::new();
            for (k, v_b) in b_obj {
                if let Some(v_a) = a_obj.get(k) {
                    if let Some(v_diff) = json_diff(v_a, v_b) {
                        diff.insert(k.clone(), v_diff);
                    }
                } else {
                    diff.insert(k.clone(), v_b.clone());
                }
            }
            for (k, _v_a) in a_obj {
                if !b_obj.contains_key(k) {
                    diff.insert(k.clone(), Value::Null);
                }
            }
            if diff.is_empty() {
                None
            } else {
                Some(Value::Object(diff))
            }
        }
        (_, b_val) => Some(b_val.clone()),
    }
}
