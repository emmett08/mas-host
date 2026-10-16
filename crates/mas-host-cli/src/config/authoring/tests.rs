use super::*;
use serde_json::json;

#[test]
fn formats_resolve_to_identical_values_with_exact_integers() {
    let json = r#"{"answer":1234567890123456789012345678901234567890,"agent":{"name":"worker","enabled":true},"steps":["first","second"],"base":255}"#;
    let toml = "answer = 1234567890123456789012345678901234567890\nbase = 0xff\nsteps = ['first', 'second']\n[agent]\nname = 'worker'\nenabled = true\n";
    let yaml = "# Comments do not enter the resolved bundle\nanswer: 1234567890123456789012345678901234567890\nbase: 0xff\nsteps: [first, second]\nagent:\n  name: worker\n  enabled: true\n";
    let expected = parse(json, Format::Json).unwrap();
    assert_eq!(parse(toml, Format::Toml).unwrap(), expected);
    assert_eq!(parse(yaml, Format::Yaml).unwrap(), expected);
    for text in ["answer: -0002", "answer: -2", "answer: -02"] {
        assert_eq!(parse(text, Format::Yaml).unwrap(), json!({"answer": -2}));
    }
}

#[test]
fn non_decimal_toml_integers_are_lossless_beyond_u64() {
    assert_eq!(
        parse("n=0xffffffffffffffffffffffffffffffff", Format::Toml).unwrap(),
        parse(
            "{\"n\":340282366920938463463374607431768211455}",
            Format::Json
        )
        .unwrap()
    );
}

#[test]
fn rejects_ambiguous_and_unsupported_yaml_before_execution() {
    for (text, message) in [
        ("x: 1\nx: 2", "duplicate"),
        ("1: value", "keys must be strings"),
        ("[a, b]: value", "keys must be strings"),
        ("x: &shared 1\ny: *shared", "anchors"),
        ("x: !custom abc", "tags"),
        ("x: !!str abc", "tags"),
        ("<<: {a: 1}", "merge keys"),
        ("---\nx: 1\n---\ny: 2", "one YAML document"),
        ("x: 1.0", "floating-point"),
        ("x: 1e20", "floating-point"),
        ("x: -.Inf", "floating-point"),
        ("x: .NaN", "floating-point"),
    ] {
        let error = parse(text, Format::Yaml).unwrap_err();
        assert!(error.contains(message), "{text:?}: {error}");
        assert!(error.contains("line"), "{error}");
    }
}

#[test]
fn yaml_core_scalars_and_quoted_values_have_explicit_semantics() {
    assert_eq!(
        parse(
            "yes: no\nx: '1.5'\ny: \"false\"\nz: null\nday: 2026-10-16\n",
            Format::Yaml
        )
        .unwrap(),
        json!({"yes":"no","x":"1.5","y":"false","z":null,"day":"2026-10-16"})
    );
    assert_eq!(
        parse("'true': value\n'<<escaped': text", Format::Yaml).unwrap(),
        json!({"true":"value","<<escaped":"text"})
    );
}

#[test]
fn rejects_lossy_toml_values_and_duplicate_keys() {
    for (text, message) in [
        ("a = 1.0", "floating-point"),
        ("a = inf", "floating-point"),
        ("a = 2026-10-16", "dates/times"),
        ("a = 07:04:39", "dates/times"),
        ("a = 1\na = 2", "duplicate"),
    ] {
        let error = parse(text, Format::Toml).unwrap_err();
        assert!(error.contains(message), "{text:?}: {error}");
        assert!(error.contains("line"), "{error}");
    }
}

#[test]
fn size_and_depth_are_bounded_in_source_and_resolved_values() {
    for format in [Format::Json, Format::Toml, Format::Yaml] {
        assert!(parse(&" ".repeat(MAX_BYTES + 1), format)
            .unwrap_err()
            .contains("SIZE"));
    }
    for depth in [31, 32, 33, 2000] {
        let nested = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(parse(&nested, Format::Yaml).is_ok(), depth <= 32);
        assert_eq!(
            parse(&format!("x={nested}"), Format::Toml).is_ok(),
            depth < 32
        );
    }
    // A short YAML source can become too large when JSON escapes its contents.
    let expanded = format!("x: '{}'", "\t".repeat(33_000));
    assert!(parse(&expanded, Format::Yaml).unwrap_err().contains("SIZE"));
}

#[test]
fn strict_json_keeps_original_rejections_and_exact_integer_semantics() {
    for text in ["{\"a\":1,\"a\":2}", "{\"a\":1.0}"] {
        assert!(parse(text, Format::Json).is_err());
    }
}

#[test]
fn authoring_integers_obey_protocol_digit_limit() {
    let exact = "9".repeat(MAX_INTEGER_DIGITS);
    let oversized = "9".repeat(MAX_INTEGER_DIGITS + 1);
    assert!(parse(&format!("n: {exact}"), Format::Yaml).is_ok());
    assert!(parse(&format!("n = {exact}"), Format::Toml).is_ok());
    assert!(parse(&format!("n: {oversized}"), Format::Yaml)
        .unwrap_err()
        .contains("INTEGER"));
    assert!(parse(&format!("n = {oversized}"), Format::Toml)
        .unwrap_err()
        .contains("INTEGER"));
    assert_eq!(
        parse(&format!("n: -{}2", "0".repeat(2000)), Format::Yaml).unwrap(),
        json!({"n":-2})
    );
}
