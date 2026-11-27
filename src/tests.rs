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

#[test]
fn rejects_source_over_the_input_budget() {
    let dir = std::env::temp_dir().join(format!("ai-review-oversize-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.to_str().unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args([
                "-C",
                repo,
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
            ])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    std::fs::write(dir.join("big.txt"), "a".repeat(MAX_SOURCE_BYTES * 4)).unwrap();
    git(&["add", "big.txt"]);
    git(&["commit", "-q", "-m", "big"]);
    let head = git_read(repo, &["rev-parse", "HEAD"], 65).unwrap();
    let sha = std::str::from_utf8(&head).unwrap().trim().to_owned();

    let (tx, rx) = std::sync::mpsc::channel();
    let path = repo.to_owned();
    std::thread::spawn(move || {
        let result = collect_source(&path, &sha, &["big.txt".into()]).map(|_| ());
        tx.send(result.map_err(|error| error.to_string())).unwrap();
    });
    let result = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("collect_source did not finish on an oversized blob");
    assert!(result.unwrap_err().contains("64 KiB"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn provider(vars: &[(&str, &str)]) -> Result<Provider, Box<dyn Error>> {
    let vars: BTreeMap<String, String> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    Provider::from_lookup(|name| vars.get(name).cloned())
}

#[test]
fn defaults_to_deepseek_and_keeps_the_legacy_key_name() {
    let deepseek = provider(&[("DEEPSEEK_API_KEY", "k")]).unwrap();
    assert_eq!(
        deepseek.endpoint,
        "https://api.deepseek.com/chat/completions"
    );
    assert_eq!(deepseek.model, "deepseek-flash");
    let body = deepseek.body(&BTreeMap::new()).unwrap();
    assert_eq!(body["thinking"]["type"], "disabled");
    assert!(provider(&[]).is_err());
    assert!(provider(&[("AI_REVIEW_API_KEY", "  ")]).is_err());
}

#[test]
fn accepts_any_openai_compatible_host_and_model() {
    let openai = provider(&[
        ("AI_REVIEW_API_KEY", "new"),
        ("DEEPSEEK_API_KEY", "old"),
        ("AI_REVIEW_BASE_URL", "https://api.openai.com/v1/"),
        ("AI_REVIEW_MODEL", "gpt-5-mini"),
    ])
    .unwrap();
    assert_eq!(openai.key, "new");
    assert_eq!(
        openai.endpoint,
        "https://api.openai.com/v1/chat/completions"
    );
    let body = openai.body(&BTreeMap::new()).unwrap();
    assert_eq!(body["model"], "gpt-5-mini");
    assert!(body.get("thinking").is_none());
    let local = provider(&[
        ("AI_REVIEW_API_KEY", "k"),
        ("AI_REVIEW_BASE_URL", "http://localhost:8000/v1"),
    ])
    .unwrap();
    assert_eq!(local.endpoint, "http://localhost:8000/v1/chat/completions");
}

#[test]
fn rejects_unsafe_base_urls() {
    for base in [
        "http://api.example.com/v1",
        "http://localhost.evil.com/v1",
        "ftp://api.example.com",
        "https://api.deepseek.com@evil.com",
        "https://api.example.com/v1?x=1",
        "https://",
    ] {
        let result = provider(&[("AI_REVIEW_API_KEY", "k"), ("AI_REVIEW_BASE_URL", base)]);
        assert!(result.is_err(), "{base} should be rejected");
    }
}
