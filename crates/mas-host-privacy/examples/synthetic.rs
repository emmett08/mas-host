//! Run safe synthetic N0 checks without model assets, credentials or side effects.
use mas_host_privacy::{
    decide, scan_synthetic, ByteRange, ExecutionCertainty, HandlingContext, HandlingProfile,
    Limits, Source,
};

fn main() {
    let limits = Limits::default();
    let context = HandlingContext {
        profile: HandlingProfile::Protected,
        policy_available: true,
        authorised: true,
        findings_permitted: false,
        suspected_exposure: false,
        execution: ExecutionCertainty::NotSent,
    };
    for (id, text) in [
        ("fixture-clean", "Synthetic ordinary text"),
        ("fixture-positive", "synthetic_secret_example"),
    ] {
        let source = Source::text(id, text, limits.max_input_bytes).expect("bounded fixture");
        let assessment = scan_synthetic(&source, limits).expect("synthetic scan");
        let decision = decide(&assessment, &source, context, limits).expect("bound handling");
        let categories: Vec<_> = assessment
            .findings()
            .iter()
            .flat_map(|finding| finding.categories.iter().map(|category| category.as_str()))
            .collect();
        println!(
            "{id}: coverage={:?}, categories={categories:?}, disposition={:?}, reason={:?}",
            assessment.completion(),
            decision.disposition,
            decision.reason
        );
    }
    let source = Source::text("fixture-unicode", "é", limits.max_input_bytes).expect("fixture");
    println!(
        "fixture-unicode: invalid byte span rejected={}",
        ByteRange::new(&source, 1, 2).is_err()
    );
}
