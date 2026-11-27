use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

const PROMPT: &str = include_str!("owasp-review.md");
const ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
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
        println!("Sends selected source to the hosted DeepSeek API. Set DEEPSEEK_API_KEY.");
        return Ok(());
    }
    let [repo, commit, paths @ ..] = args.as_slice() else {
        return Err("expected REPOSITORY COMMIT FILE [FILE ...]; see --help".into());
    };
    let files = collect_source(repo, commit, paths)?;
    let key = std::env::var("DEEPSEEK_API_KEY").map_err(|_| "set DEEPSEEK_API_KEY")?;
    if key.trim().is_empty() {
        return Err("set DEEPSEEK_API_KEY to a nonempty value".into());
    }
    let review = request_review(&key, &files)?;
    let report = json!({
        "reviewed_commit": commit,
        "reviewed_paths": files.keys().collect::<Vec<_>>(),
        "coverage": review.coverage,
        "findings": review.findings,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
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
        .env_remove("DEEPSEEK_API_KEY")
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

fn request_review(key: &str, files: &BTreeMap<String, String>) -> Result<Review, Box<dyn Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .json(&json!({
            "model": "deepseek-flash",
            "thinking": {"type": "disabled"},
            "response_format": {"type": "json_object"},
            "max_tokens": 4096,
            "messages": [
                {"role": "system", "content": PROMPT},
                {"role": "user", "content": serde_json::to_string(files)?}
            ]
        }))
        .send()?;
    if !response.status().is_success() {
        return Err(format!("DeepSeek returned HTTP {}", response.status().as_u16()).into());
    }
    let mut body = Vec::new();
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("DeepSeek response exceeded the size limit".into());
    }
    parse_review(&body, files)
}

fn parse_review(body: &[u8], files: &BTreeMap<String, String>) -> Result<Review, Box<dyn Error>> {
    let completion: Completion =
        serde_json::from_slice(body).map_err(|_| "invalid DeepSeek response envelope")?;
    let [choice] = completion.choices.as_slice() else {
        return Err("expected exactly one DeepSeek result".into());
    };
    if choice.finish_reason != "stop" {
        return Err("DeepSeek did not finish the review".into());
    }
    let content = choice
        .message
        .content
        .as_deref()
        .ok_or("DeepSeek returned no review")?;
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
