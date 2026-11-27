# AI Security Review

A Rust CLI for advisory source-code review against the
[OWASP Web Application Top 10:2025](https://top10.owasp.org/2025/).
It reads selected files at an exact Git commit, sends their text to an
OpenAI-compatible chat completions API, and validates a JSON report against the
supplied paths, line numbers, and OWASP categories. The CLI treats reviewed code
as data and gives the model zero tools. Use the report to guide human review.

DeepSeek is the default. You can point the CLI at an approved hosted provider or
a compatible internal model gateway. A self-hosted runner controls where the CLI
runs. The model endpoint still receives every selected path and its contents.
Use the synthetic fixture first and send company source only to a provider
approved for that repo.

## GitHub-hosted CI and Docker images

GitHub Actions runs Rust checks and a synthetic end-to-end review on pull
requests and pushes to `main`. The smoke test uses a local stub for the model
API and needs no API key. After the tests pass on `main`, Actions builds and
smoke-tests a Linux Docker image, then pushes it to
`ghcr.io/solutionforest/ai-security-review:<commit-sha>`. Download access
depends on the GHCR package visibility and permissions.

### Build and run the Rust reviewer

On Linux with Docker, build the image and run the Rust CLI inside it:

```sh
docker build -t ai-security-review:local .
docker run --rm ai-security-review:local --help
```

Docker compiles the CLI in a Rust build stage. The final image runs the compiled
binary directly. To use the Rust binary without Docker, see
[Run locally or review a separate repo](#run-locally-or-review-a-separate-repo).

### Optional end-to-end smoke test

The smoke-test helper uses Python 3 on the host to start a mock model API and
verify the Rust CLI's JSON report. It uses synthetic source and needs no API
key. This is the only step here that requires Python.

```sh
python3 scripts/smoke-test.py --image ai-security-review:local
```

The public workflow uses only synthetic source and a local stub. Run model
reviews that use real company source or provider credentials from a separate
private automation repo.

## 1. Prepare a Linux runner for private reviews

1. Provision a dedicated, disposable Linux VM. Use an unprivileged account for
   the runner. Give the VM access to GitHub Actions, the Rust toolchain and crate
   sources needed by the build, and the selected model endpoint. Use the
   [GitHub runner network requirements](https://docs.github.com/en/actions/reference/runners/self-hosted-runners#communication)
   to configure outbound access, including access for artifact upload.
2. Install Git, a C compiler, CMake, CA certificates, and
   [rustup](https://rustup.rs/) under the runner account. On Ubuntu, the
   system prerequisites can be installed with
   `sudo apt-get install git build-essential cmake ca-certificates`.
   From the runner account, install and verify the toolchain:

   ```sh
   rustup toolchain install 1.100.0 --profile minimal
   cargo +1.100.0 --version
   ```

3. In your private automation repo, open **Settings → Actions → Runners →
   New self-hosted runner**. Select **Linux** and the VM's architecture.
   Run the download and checksum commands GitHub provides on the VM.
   Run its generated `./config.sh` command with
   `--labels ai-review --ephemeral` appended. The generated token registers
   the runner with GitHub. Keep it private and use it only on the VM.
4. From the runner directory, start `./run.sh` as the runner account and
   leave it running until the job picks it up. Verify GitHub shows the runner
   as **Idle** with the `self-hosted`, `linux`, and `ai-review` labels.
   An ephemeral runner takes one job and deregisters. Destroy the VM after the
   run and register a fresh one for the next job.
   [GitHub registration](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/add-runners) ·
   [Ephemeral runners](https://docs.github.com/en/actions/reference/runners/self-hosted-runners#ephemeral-runners-for-autoscaling)

For an organization-managed runner, use a dedicated runner group restricted
to your private automation repo. Restrict its workflow access to the protected
review workflow where available, and use the group along with the
`ai-review` label in `runs-on`.
[Runner group access](https://docs.github.com/en/enterprise-cloud@latest/actions/how-tos/manage-runners/self-hosted-runners/manage-access)

## 2. Set the provider and API secret

1. In your private automation repo, open **Settings → Environments → New
   environment** and name it `security-review`.
2. Under **Deployment branches and tags**, choose **Selected branches and
   tags** and add a **Branch** rule for `main`. Protect `main` with your
   usual pull request review rules.
3. Under **Environment secrets**, select **Add secret**. Set the name to
   `AI_REVIEW_API_KEY` and paste the provider API key as the value. This is the
   model API credential. GitHub's runner registration token is used in step 1
   above. Keep the API key out of source, YAML, command arguments, and logs.
4. For a non-default provider, add `AI_REVIEW_BASE_URL` and `AI_REVIEW_MODEL`
   under **Environment variables**. Confirm the hostname and model together
   before using a provider key. Private repository environments require a
   [GitHub plan that supports them](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments#environment-secrets).
   [GitHub environment setup](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments)

| Setting | Default | Value |
| --- | --- | --- |
| `AI_REVIEW_API_KEY` | Required | Bearer token. The CLI also accepts `DEEPSEEK_API_KEY` as a legacy fallback. The workflow below uses the primary name. |
| `AI_REVIEW_BASE_URL` | `https://api.deepseek.com` | Base URL, for example `https://llm.internal.example/v1`. The CLI appends `/chat/completions`. Use HTTPS, or HTTP on `localhost` or `127.0.0.1` for a local test. |
| `AI_REVIEW_MODEL` | `deepseek-flash` | Exact model ID accepted by the endpoint. |

The endpoint must support JSON object mode via
`response_format: {"type": "json_object"}`.
[DeepSeek JSON output](https://api-docs.deepseek.com/guides/json_mode/)

## 3. Add and run the manual workflow

Copy the example into `.github/workflows/ai-security-review.yml` in your
private automation repo and merge it to that repo's default branch. This example assumes the default branch is
`main` and the runner uses the labels from step 1. It builds the reviewer
without the API key and then gives the key only to the fixture step. Keep the
manual trigger for the initial pilot.
[Manual workflow runs](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)

```yaml
name: AI Security Review
on: workflow_dispatch

permissions:
  contents: read

jobs:
  review:
    if: github.ref == 'refs/heads/main'
    runs-on: [self-hosted, linux, ai-review]
    environment: security-review
    timeout-minutes: 20
    steps:
      - name: Check out the reviewer
        uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
        with:
          repository: solutionforest/ai-security-review
          ref: b443286d1569e85fcbaa14ef7a83f5fc077806e5
          persist-credentials: false
      - name: Build the reviewer
        run: cargo +1.100.0 build --locked --release --bin ai-security-review
      - name: Review the synthetic fixture
        env:
          AI_REVIEW_API_KEY: ${{ secrets.AI_REVIEW_API_KEY }}
          AI_REVIEW_BASE_URL: ${{ vars.AI_REVIEW_BASE_URL }}
          AI_REVIEW_MODEL: ${{ vars.AI_REVIEW_MODEL }}
        run: |
          target/release/ai-security-review . "$(git rev-parse HEAD)" fixtures/policy.txt > review.json
      - name: Upload the report
        uses: actions/upload-artifact@bbbca2ddaa5d8feaa63e36b76fdaad77386f024f # v7.0.0
        with:
          name: ai-security-review
          path: review.json
          if-no-files-found: error
          retention-days: 3
```

On GitHub, select **Actions → AI Security Review → Run workflow**, choose
`main`, and open the run. Download **ai-security-review** under **Artifacts**.
Check that `review.json` records the reviewed commit, the model, the fixture
path, and coverage entries for all ten OWASP categories. A valid report can
contain an empty findings list. Keep the report private and triage any findings
before acting on them.

If the workflow stays queued, check that a fresh runner is online and matches
all labels. If the review step reports a missing key, check the secret name and
environment branch rule. HTTP 401 usually means the model credential needs
checking. HTTP 404 can indicate an incorrect base URL or model ID. For an
invalid report, verify the provider supports JSON object mode and retry with the
synthetic fixture.

## Run locally or review a separate repo

Build from a trusted reviewer checkout with
`cargo +1.100.0 build --locked --release --bin ai-security-review`.
For a local smoke run, set `AI_REVIEW_API_KEY` in your shell using your secret
manager and run this from the reviewer checkout:

```sh
target/release/ai-security-review . "$(git rev-parse HEAD)" fixtures/policy.txt > review.json
```

For real source, pass a separate local Git checkout, the full commit SHA, and
1–20 explicit UTF-8 paths relative to that repo. The total limit is 64 KiB. Review
target files as data. Build and run the reviewer from its trusted checkout.

```sh
target/release/ai-security-review ../my-app "$TARGET_COMMIT" src/routes.rs src/auth.rs > review.json
```

For the next stage after the fixture, choose one private pilot repo and a small
set of source files approved for the model. If Actions needs to fetch another
private repo, give that checkout narrowly scoped read access. The automatically
issued `GITHUB_TOKEN` is scoped to the workflow's repository.
[GitHub token scope](https://docs.github.com/en/actions/concepts/security/github_token)

See [SKILLS.md](SKILLS.md) for additional review guidance and [AGENTS.md](AGENTS.md)
for maintainer and coding-agent instructions.
