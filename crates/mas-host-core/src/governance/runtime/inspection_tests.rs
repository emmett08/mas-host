use super::*;

#[test]
fn decoded_keys_and_values_are_exact_bounded_text_sources() {
    let value: Value =
        serde_json::from_str(r#"{"\u00e9\u0301":"a\nb","items":["\ud83d\ude80",1,true,null]}"#)
            .unwrap();
    let mut units = Vec::new();
    text_units(&value, &mut units).unwrap();
    assert_eq!(units, vec!["items", "🚀", "é\u{301}", "a\nb"]);
    assert!(text_units(&json!(vec![""; MAX_TEXT_UNITS + 1]), &mut Vec::new()).is_err());
}

#[test]
fn all_numeric_and_empty_values_have_no_unchecked_text() {
    for value in [json!(1), json!([1, true, null]), json!({}), json!([])] {
        let mut units = Vec::new();
        text_units(&value, &mut units).unwrap();
        assert!(units.is_empty());
    }
}
