use super::*;
use serde_json::Value;

fn completion(review: &Value, finish_reason: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "choices": [{
            "finish_reason": finish_reason,
            "message": {"content": review.to_string()}
        }]
    }))
    .unwrap()
}

fn fixture() -> (Value, BTreeMap<String, String>) {
    let coverage: BTreeMap<_, _> = (1..=10)
        .map(|id| (format!("A{id:02}:2025"), "insufficient_evidence"))
        .collect();
    let review = json!({
        "coverage": coverage,
        "findings": [{
            "category": "A01:2025",
            "path": "fixtures/policy.txt",
            "line": 2,
            "status": "needs_validation",
            "evidence": "Caller identity is defined outside the supplied scope.",
            "impact": "Authorization depends on the identity check.",
            "remediation": "Verify that the caller is authenticated.",
            "missing_fact": "The upstream authentication policy."
        }]
    });
    let files = BTreeMap::from([("fixtures/policy.txt".into(), "policy\nidentity\n".into())]);
    (review, files)
}

#[test]
fn accepts_a_complete_report_with_a_source_location() {
    let (review, files) = fixture();
    let parsed = parse_review(&completion(&review, "stop"), &files).unwrap();
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(parsed.coverage.len(), 10);
}

#[test]
fn rejects_missing_coverage_and_truncated_output() {
    let (mut review, files) = fixture();
    assert!(parse_review(&completion(&review, "length"), &files).is_err());
    review["coverage"]
        .as_object_mut()
        .unwrap()
        .remove("A10:2025");
    assert!(parse_review(&completion(&review, "stop"), &files).is_err());
}

#[test]
fn rejects_invalid_findings() {
    let (review, files) = fixture();
    for (key, value) in [
        ("category", json!("A01:2021")),
        ("line", json!(3)),
        ("path", json!("other.txt")),
        ("missing_fact", Value::Null),
    ] {
        let mut invalid = review.clone();
        invalid["findings"][0][key] = value;
        assert!(parse_review(&completion(&invalid, "stop"), &files).is_err());
    }
}

#[test]
fn reads_committed_source_and_rejects_unscoped_inputs() {
    let repo = env!("CARGO_MANIFEST_DIR");
    let head = git_read(repo, &["rev-parse", "HEAD"], 65).unwrap();
    let sha = std::str::from_utf8(&head).unwrap().trim();
    let files = collect_source(repo, sha, &["README.md".into()]).unwrap();
    assert!(!files["README.md"].is_empty());
    for invalid in ["../README.md", "missing-file", "/README.md"] {
        assert!(collect_source(repo, sha, &[invalid.into()]).is_err());
    }
    assert!(collect_source(repo, "HEAD", &["README.md".into()]).is_err());
    assert!(collect_source(repo, sha, &["README.md".into(), "README.md".into()]).is_err());
    assert!(git_read(repo, &["rev-parse", "HEAD"], 1).is_err());
}
