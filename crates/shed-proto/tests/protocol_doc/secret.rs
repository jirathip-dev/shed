use serde_json::{Value, json, to_value};
use shed_proto::{Method, SavedSecret, SecretName, SecretSet};

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    let name = || "BOAT_API_KEY".to_owned();
    Some(match method {
        Method::SecretSet => (
            to_value(SecretSet {
                name: name(),
                value: "bk_0123456789abcdef".into(),
            })
            .unwrap(),
            json!({}),
        ),
        Method::SecretLs => (
            json!({}),
            to_value(vec![SavedSecret {
                name: name(),
                saved_at: Some(1_762_416_000),
            }])
            .unwrap(),
        ),
        Method::SecretRm => (to_value(SecretName { name: name() }).unwrap(), json!({})),
        _ => return None,
    })
}

#[test]
fn a_set_s_debug_leaves_the_value_out() {
    let set = SecretSet {
        name: "BOAT_API_KEY".into(),
        value: "bk_0123456789abcdef".into(),
    };
    let shown = format!("{set:?}");
    assert!(
        shown.contains("BOAT_API_KEY") && !shown.contains("bk_0123"),
        "{shown}"
    );
}
