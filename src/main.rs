use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

const PROMPT: &str = include_str!("owasp-review.md");
const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-flash";
const SECRET_VARS: [&str; 2] = ["AI_REVIEW_API_KEY", "DEEPSEEK_API_KEY"];
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_FILES: usize = 20;

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    finish_reason: String,
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    Reviewed,
    NotApplicable,
    InsufficientEvidence,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum FindingStatus {
    SourceSupported,
    NeedsValidation,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    category: String,
    path: String,
    line: usize,
    status: FindingStatus,
    evidence: String,
    impact: String,
    remediation: String,
    missing_fact: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Review {
    coverage: BTreeMap<String, Coverage>,
    findings: Vec<Finding>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Review failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!("Usage: ai-security-review REPOSITORY COMMIT FILE [FILE ...]");
        println!("Sends selected source to an OpenAI-compatible chat completions API.");
        println!("Set AI_REVIEW_API_KEY (or DEEPSEEK_API_KEY). Optional: AI_REVIEW_BASE_URL");
        println!("(default {DEFAULT_BASE_URL}) and AI_REVIEW_MODEL (default {DEFAULT_MODEL}).");
        return Ok(());
    }
    let [repo, commit, paths @ ..] = args.as_slice() else {
        return Err("expected REPOSITORY COMMIT FILE [FILE ...]; see --help".into());
    };
    let files = collect_source(repo, commit, paths)?;
    let provider = Provider::from_lookup(|name| std::env::var(name).ok())?;
    let review = request_review(&provider, &files)?;
    let report = json!({
        "model": provider.model,
        "reviewed_commit": commit,
        "reviewed_paths": files.keys().collect::<Vec<_>>(),
        "coverage": review.coverage,
        "findings": review.findings,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct Provider {
    endpoint: String,
    key: String,
    model: String,
    deepseek: bool,
}

impl Provider {
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, Box<dyn Error>> {
        let set = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
        let key = SECRET_VARS
            .iter()
            .find_map(|name| set(name))
            .ok_or("set AI_REVIEW_API_KEY (or DEEPSEEK_API_KEY) to a nonempty value")?;
        let base = set("AI_REVIEW_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into());
        let base = base.trim().trim_end_matches('/');
        let host = base
            .strip_prefix("https://")
            .or_else(|| {
                base.strip_prefix("http://")
                    .filter(|rest| is_loopback(rest.split(['/', ':']).next().unwrap_or("")))
            })
            .ok_or("AI_REVIEW_BASE_URL must use https:// (http:// only for localhost)")?
            .split(['/', ':'])
            .next()
            .unwrap_or("");
        if host.is_empty()
            || base.contains(['?', '#', '@'])
            || base.chars().any(char::is_whitespace)
        {
            return Err(
                "AI_REVIEW_BASE_URL must be a plain base URL such as https://host/v1".into(),
            );
        }
        Ok(Self {
            deepseek: host == "api.deepseek.com",
            endpoint: format!("{base}/chat/completions"),
            key,
            model: set("AI_REVIEW_MODEL")
                .map(|model| model.trim().to_owned())
                .unwrap_or_else(|| DEFAULT_MODEL.into()),
        })
    }

    fn body(&self, files: &BTreeMap<String, String>) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut body = json!({
            "model": self.model,
            "response_format": {"type": "json_object"},
            "max_tokens": 4096,
            "messages": [
                {"role": "system", "content": PROMPT},
                {"role": "user", "content": serde_json::to_string(files)?}
            ]
        });
        if self.deepseek {
            // DeepSeek-only switch; other providers may reject unknown fields.
            body["thinking"] = json!({"type": "disabled"});
        }
        Ok(body)
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1")
}

fn collect_source(
    repo: &str,
    commit: &str,
    paths: &[String],
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    if !matches!(commit.len(), 40 | 64)
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("supply a full lowercase Git commit SHA".into());
    }
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err("select between 1 and 20 source files".into());
    }
    if git_read(repo, &["cat-file", "-t", commit], 16)? != b"commit\n" {
        return Err("the requested SHA must identify a commit".into());
    }

    let mut files = BTreeMap::new();
    let mut remaining = MAX_SOURCE_BYTES;
    for path in paths {
        if path.contains(['\\', ':'])
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err("supply normal repository-relative file paths".into());
        }
        if files.contains_key(path) {
            return Err("source paths must be unique".into());
        }
        let object = format!("{commit}:{path}");
        let bytes = git_read(repo, &["cat-file", "blob", &object], remaining)?;
        remaining -= bytes.len();
        if bytes.contains(&0) {
            return Err("selected source includes a binary file".into());
        }
        files.insert(
            path.clone(),
            String::from_utf8(bytes).map_err(|_| "source must be UTF-8")?,
        );
    }
    Ok(files)
}

fn git_read(repo: &str, args: &[&str], limit: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut child = Command::new("git")
        .args(["--no-replace-objects", "-C", repo])
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .env_remove(SECRET_VARS[0])
        .env_remove(SECRET_VARS[1])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let output = child.stdout.take().expect("Git stdout is piped");
    let result = output.take((limit + 1) as u64).read_to_end(&mut bytes);
    let status = child.wait()?;
    result?;
    if bytes.len() > limit {
        return Err("selected source exceeds the 64 KiB input budget".into());
    }
    if !status.success() {
        return Err("Git could not read the requested commit or source blob".into());
    }
    Ok(bytes)
}

fn request_review(
    provider: &Provider,
    files: &BTreeMap<String, String>,
) -> Result<Review, Box<dyn Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .post(&provider.endpoint)
        .bearer_auth(&provider.key)
        .json(&provider.body(files)?)
        .send()
        .map_err(|_| "could not reach the model API")?;
    if !response.status().is_success() {
        return Err(format!("model API returned HTTP {}", response.status().as_u16()).into());
    }
    let mut body = Vec::new();
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("model API response exceeded the size limit".into());
    }
    parse_review(&body, files)
}

fn parse_review(body: &[u8], files: &BTreeMap<String, String>) -> Result<Review, Box<dyn Error>> {
    let completion: Completion =
        serde_json::from_slice(body).map_err(|_| "invalid model API response envelope")?;
    let [choice] = completion.choices.as_slice() else {
        return Err("expected exactly one model result".into());
    };
    if choice.finish_reason != "stop" {
        return Err("the model did not finish the review".into());
    }
    let content = choice
        .message
        .content
        .as_deref()
        .ok_or("the model returned no review")?;
    let review: Review = serde_json::from_str(content).map_err(|_| "invalid review JSON")?;
    let expected: BTreeSet<_> = (1..=10).map(|id| format!("A{id:02}:2025")).collect();
    if review.coverage.keys().ne(expected.iter()) {
        return Err("coverage must include all ten OWASP Top 10:2025 category IDs".into());
    }
    for finding in &review.findings {
        let line_count = files
            .get(&finding.path)
            .map_or(0, |source| source.lines().count());
        if !expected.contains(&finding.category) || finding.line == 0 || finding.line > line_count {
            return Err("a finding contains an invalid category or source location".into());
        }
        if [&finding.evidence, &finding.impact, &finding.remediation]
            .iter()
            .any(|field| field.trim().is_empty())
        {
            return Err("a finding is missing evidence, impact, or remediation".into());
        }
        if matches!(finding.status, FindingStatus::NeedsValidation)
            && finding
                .missing_fact
                .as_deref()
                .is_none_or(|fact| fact.trim().is_empty())
        {
            return Err("a finding needs a specific missing fact for validation".into());
        }
    }
    Ok(review)
}

#[cfg(test)]
mod tests;
