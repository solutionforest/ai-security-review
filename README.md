# AI Security Review

A Rust CLI for advisory, source-only reviews against the
[OWASP Web Application Top 10:2025](https://top10.owasp.org/2025/).
It reads selected Git blobs at an immutable commit and sends them to the
hosted DeepSeek API. It does not execute the reviewed application or give
the model tools.

## Build and test

Use Rust 1.100.0, Git, a C compiler, and CMake. Build on Linux for the
runner's architecture.

```sh
cargo +1.100.0 test --locked
cargo +1.100.0 build --locked --release --bin ai-security-review
```

## First run on a self-hosted runner

1. Register a disposable Linux runner for the pilot and restrict it to the
   protected review workflow. Use the `ai-review` label when routing the
   manual job to it.
2. Check out the trusted reviewer ref and build it using the command above.
   Give the job read-only GitHub permissions. Use the reviewer's synthetic
   fixture for the first run. For actual reviews, pass a separate checkout of
   the target repo and a maintainer-selected commit and file list. Keep target
   content out of workflow instructions and executable steps.
3. Set the DeepSeek key as the GitHub environment secret `DEEPSEEK_API_KEY` on
   an environment restricted to the protected workflow's branch. Inject the
   secret only into the step that invokes the CLI. From the reviewer checkout,
   run the fixture with:

   ```sh
   target/release/ai-security-review . "$(git rev-parse HEAD)" fixtures/policy.txt > review.json
   ```

4. Upload `review.json` as a short-lived private artifact only after a
   successful review. Use human triage before acting on findings.

The CLI accepts 1–20 explicit repository-relative UTF-8 file paths with up to
64 KiB of content in total. Pass only files approved for the hosted model.
DeepSeek receives the selected paths and text, including when Actions runs
on your own VM. Keep source, credentials, and reports out of workflow logs.
A report covers the selected files at the reported SHA and should be handled
as sensitive. Failed requests, incomplete model output, and invalid reports
return a nonzero exit code.

The first version uses DeepSeek Flash with the provider's JSON output mode.
The repository embeds an original review prompt and the official OWASP
category names. It does not bundle any of the evaluated third-party skills.
