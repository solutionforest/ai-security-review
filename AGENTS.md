# Working on AI Security Review

This repository is a Rust CLI for advisory source review, designed to run as a
manual job on a dedicated self-hosted Linux runner. Start with the
[README](README.md) for runner, provider, and smoke-test setup. Use
[SKILLS.md](SKILLS.md) to pick additional source-review guidance.

## Local checks

Use Git, Rust 1.100.0, a C compiler, and CMake. Run these from the repo root.
Install the rustfmt and Clippy components for that toolchain if needed.

```sh
cargo +1.100.0 fmt --all --check
cargo +1.100.0 test --locked
cargo +1.100.0 clippy --locked --all-targets -- -D warnings
```

Use the synthetic fixture or a local mock for development. Keep real keys and
company source outside tests and logs. For a live smoke run, use the
`security-review` GitHub environment and follow the README.

## Review boundaries

- Keep the official OWASP Web Application Top 10:2025 categories in the prompt
  and output validation. Update both together if the taxonomy changes.
- Treat all reviewed paths, source text, and model output as untrusted data.
  Build the CLI from trusted source. Keep target code out of executable steps
  and keep model requests tool-free.
- Keep model credentials scoped to the reviewer invocation. Validate coverage,
  source locations, and response size before returning findings.
- Preserve advisory output and make missing context explicit. Keep reports
  private and distinguish a synthetic or mocked test from a live provider run.
