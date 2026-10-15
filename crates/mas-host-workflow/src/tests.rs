use crate::{execute_with, validate, Result, Runtime};
use mas_host_sdk::Request;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Default)]
struct Fake {
    batches: Vec<Vec<Request>>,
}
impl Runtime for Fake {
    fn requests(&mut self, requests: &[Request]) -> Result<Vec<Value>> {
        self.batches.push(requests.to_vec());
        requests.iter().map(|r| match r.target.as_str() {
            "increment" => {
                let n = r.input["n"].as_u64().unwrap() + 1;
                Ok(json!({"n":n,"done":n >= 3}))
            }
            "double" => Ok(json!({"value":r.input["n"].as_u64().unwrap() * 2})),
            "square" => Ok(json!({"value":r.input["n"].as_u64().unwrap().pow(2)})),
            "sum" => Ok(json!({"total":r.input["total"].as_u64().unwrap() + r.input["n"].as_u64().unwrap()})),
            _ => Ok(json!({"target":r.target,"input":r.input})),
        }).collect()
    }
}
fn request(target: &str) -> Value {
    json!({"type":"request","key":target,"target":target,"input":{"$previous":""}})
}
fn targets(runtime: &Fake) -> Vec<Vec<String>> {
    runtime
        .batches
        .iter()
        .map(|b| b.iter().map(|r| r.target.clone()).collect())
        .collect()
}

#[test]
fn nested_parallel_sequences_share_ready_frontiers_and_preserve_order() {
    let plan = json!({"type":"parallel","steps":[
        {"type":"sequence","steps":[request("a"),request("a2")]},
        {"type":"parallel","steps":[
            {"type":"sequence","steps":[request("b"),request("b2")]},request("c")
        ]}
    ]});
    let mut fake = Fake::default();
    let result = execute_with(&mut fake, &plan, json!({"x":7})).unwrap();
    assert_eq!(targets(&fake), vec![vec!["a", "b", "c"], vec!["a2", "b2"]]);
    assert_eq!(result["results"][0]["target"], "a2");
    assert_eq!(result["results"][0]["input"]["target"], "a");
    assert_eq!(result["results"][1]["results"][0]["target"], "b2");
    assert_eq!(result["results"][1]["results"][1]["input"], json!({"x":7}));
}

#[test]
fn repeat_named_definition_and_branch_carry_outputs_and_unique_replay_keys() {
    let plan = json!({"type":"sequence", "definitions":{
        "improve":{"type":"repeat","max_iterations":4,"until":{"$previous":"/done"},"body":{
            "type":"request","key":"improve","target":"increment","input":{"n":{"$previous":"/n"},"index":{"$iteration":"/index"}}
        }}
    },"steps":[{"type":"call","name":"improve"},{"type":"branch","condition":{"$previous":"/done"},
        "then":{"type":"value","output":{"initial":{"$input":"/n"},"final":{"$previous":"/n"}}},
        "else":request("unreachable")
    }]});
    let mut first = Fake::default();
    let mut replay = Fake::default();
    let expected = json!({"initial":0,"final":3});
    assert_eq!(
        execute_with(&mut first, &plan, json!({"n":0})).unwrap(),
        expected
    );
    assert_eq!(
        execute_with(&mut replay, &plan, json!({"n":0})).unwrap(),
        expected
    );
    let keys = |r: &Fake| {
        r.batches
            .iter()
            .flatten()
            .map(|r| r.key.clone())
            .collect::<Vec<_>>()
    };
    let first_keys = keys(&first);
    assert_eq!(first_keys, keys(&replay));
    assert_eq!(first_keys.iter().collect::<BTreeSet<_>>().len(), 3);
    assert!(first_keys.iter().all(|k| k.len() <= 128));
    assert_eq!(
        first
            .batches
            .iter()
            .map(|b| b[0].input["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        targets(&first),
        vec![vec!["increment"], vec!["increment"], vec!["increment"]]
    );
}

#[test]
fn foreach_parallel_matches_direct_arithmetic_oracle_and_chunks_frontier() {
    let plan = json!({"type":"foreach","mode":"parallel","items":{"$input":"/numbers"},"body":{
        "type":"parallel","steps":[
            {"type":"request","key":"twice","target":"double","input":{"n":{"$item":""}}},
            {"type":"request","key":"squared","target":"square","input":{"n":{"$item":""}}}
        ]
    }});
    let numbers: Vec<_> = (1u64..=10).collect();
    let mut fake = Fake::default();
    let result = execute_with(&mut fake, &plan, json!({"numbers":numbers})).unwrap();
    let expected: Vec<_> = (1u64..=10)
        .map(|n| json!({"results":[{"value":2*n},{"value":n*n}]}))
        .collect();
    assert_eq!(result, json!({"results":expected}));
    assert_eq!(
        fake.batches.iter().map(Vec::len).collect::<Vec<_>>(),
        [8, 8, 4]
    );
}

#[test]
fn sequential_foreach_passes_accumulator_and_empty_array_returns_no_requests() {
    let plan = json!({"type":"foreach","mode":"sequence","items":{"$input":"/numbers"},"body":{
        "type":"request","key":"add","target":"sum","input":{"n":{"$item":""},"total":{"$previous":"/total"}}
    }});
    let mut fake = Fake::default();
    assert_eq!(
        execute_with(&mut fake, &plan, json!({"numbers":[1,2,3],"total":0})).unwrap(),
        json!({"results":[{"total":1},{"total":3},{"total":6}]})
    );
    let mut empty = Fake::default();
    assert_eq!(
        execute_with(&mut empty, &plan, json!({"numbers":[],"total":0})).unwrap(),
        json!({"results":[]})
    );
    assert!(empty.batches.is_empty());
}

#[test]
fn validation_rejects_hidden_unknown_targets_cycles_and_malformed_nodes() {
    let agents = BTreeSet::from(["good".to_string()]);
    let bad = [
        json!({"type":"branch","condition":false,"then":request("missing"),"else":request("good")}),
        json!({"type":"call","name":"a","definitions":{"a":{"type":"call","name":"b"},"b":{"type":"call","name":"a"}}}),
        json!({"type":"call","name":"missing"}),
        json!({"type":"value","output":{},"extra":true}),
        json!({"type":"value","output":{"v":{"$input":"/bad~2"}}}),
        json!({"type":"value","output":{"v":{"$input":"/x","other":1}}}),
        json!({"type":"branch","condition":1,"then":request("good"),"else":request("good")}),
        json!({"type":"repeat","max_iterations":0,"body":request("good"),"until":true}),
        json!({"type":"sequence","steps":[]}),
        json!({"type":"request","target":"good","input":{}}),
        json!({"type":"value","output":{"float":1.5}}),
    ];
    for plan in bad {
        assert!(validate(&plan, &agents).is_err(), "accepted {plan}");
    }
    assert!(validate(&request("good"), &agents).is_ok());
    assert!(validate(&json!({"type":"value","output":{},"definitions":{"unused":{"type":"call","name":"missing"}}}),&agents).is_err());
}

#[test]
fn pointers_escape_correctly_and_missing_data_never_invokes_actor() {
    let plan = json!({"type":"request","key":"pointer","target":"echo","input":{"value":{"$input":"/a~1b/~0/🧠"}}});
    let mut fake = Fake::default();
    assert_eq!(
        execute_with(&mut fake, &plan, json!({"a/b":{"~":{"🧠":42}}})).unwrap()["input"]["value"],
        42
    );
    let mut absent = Fake::default();
    assert!(execute_with(&mut absent, &plan, json!({})).is_err());
    assert!(absent.batches.is_empty());
}

#[test]
fn exhausted_repeat_is_failure_and_runtime_cannot_omit_outputs() {
    let plan = json!({"type":"repeat","max_iterations":2,"body":request("echo"),"until":false});
    let mut fake = Fake::default();
    assert!(execute_with(&mut fake, &plan, json!({}))
        .unwrap_err()
        .contains("max_iterations"));
    assert_eq!(fake.batches.len(), 2);
    struct Broken;
    impl Runtime for Broken {
        fn requests(&mut self, _: &[Request]) -> Result<Vec<Value>> {
            Ok(vec![])
        }
    }
    assert!(execute_with(&mut Broken, &request("echo"), json!({}))
        .unwrap_err()
        .contains("wrong number"));
}

#[test]
fn static_expansion_and_materialised_values_have_independent_limits() {
    let mut definitions = Map::new();
    for i in 0..65 {
        definitions.insert(
            format!("d{i}"),
            if i == 64 {
                json!({"type":"value","output":{}})
            } else {
                json!({"type":"call","name":format!("d{}",i+1)})
            },
        );
    }
    let plan = json!({"type":"call","name":"d0","definitions":definitions});
    assert!(validate(&plan, &BTreeSet::new())
        .unwrap_err()
        .contains("depth"));
    let plan =
        json!({"type":"value","output":{"left":{"$input":"/text"},"right":{"$input":"/text"}}});
    let mut fake = Fake::default();
    assert!(
        execute_with(&mut fake, &plan, json!({"text":"x".repeat(40_000)}))
            .unwrap_err()
            .contains("65536")
    );
    assert!(fake.batches.is_empty());
}

#[test]
fn dynamic_wrong_types_fail_before_any_following_request() {
    for plan in [
        json!({"type":"branch","condition":{"$input":"/condition"},"then":request("one"),"else":request("two")}),
        json!({"type":"request","key":"scalar","target":"echo","input":{"$input":"/condition"}}),
    ] {
        let mut fake = Fake::default();
        assert!(execute_with(&mut fake, &plan, json!({"condition":1})).is_err());
        assert!(fake.batches.is_empty());
    }
}
