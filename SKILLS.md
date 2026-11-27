# Security review skills

This repository's Rust CLI embeds its own source-only
[OWASP Top 10:2025 prompt](src/owasp-review.md). The skills below are optional
references for humans and coding agents reviewing the CLI, the runner setup,
or a target application's source. The CLI does not load skill files at runtime.

## Choose guidance for the change

| Review area | Suggested guidance | What to check |
| --- | --- | --- |
| Application source | [UnitOneAI secure-code-review](https://github.com/UnitOneAI/SecuritySkills/blob/main/skills/appsec/secure-code-review/SKILL.md) | Trace input validation and authorization through their callers. Use ASVS or CWE as supplementary references and map the final report to the official OWASP Web Top 10:2025 IDs. UnitOneAI's dedicated web checklist uses the 2021 edition. |
| Changed files and related code | [Red Hat differential-review](https://github.com/RedHatProductSecurity/prodsec-skills/blob/main/module/skills/differential-review/SKILL.md) | Inspect affected call paths, security assumptions, relevant tests, and gaps in the provided source. |
| Runner and supply chain | [UnitOneAI pipeline-security](https://github.com/UnitOneAI/SecuritySkills/blob/main/skills/devsecops/pipeline-security/SKILL.md) | Inspect workflow triggers, runner access, token scopes, dependency and action pins, and artifact permissions. Review where the model token and source can flow. |
| LLM, MCP, or RAG application code | [llm-security-skills catalog](https://github.com/fevziegeyurtsevenler/llm-security-skills#the-skills) | Use the static checklist guidance for model input trust, tool permissions, output handling, and retrieval isolation. The upstream LLM checklist uses the 2025 edition. Consult the official [OWASP LLM Top 10:2026](https://genai.owasp.org/resource/owasp-genai-llm-top-10-2026/) for current LLM category mappings. |

For dependency advisories, use [RustSec](https://rustsec.org/) alongside the AI
review. For the reviewer itself, trace the path from Git blobs to the request
and from the model response to the artifact. Check the size limits, credential
handling, endpoint validation, and error behavior.

Read the relevant skill before use. Adopt only reviewed static-review guidance,
pin the upstream commit when copying excerpts, and check the license of each
file. Keep active testing and any upstream instructions that execute target code
outside this source-only workflow. Prefer a short list of evidence-backed
findings with a concrete fix and flag anything that requires deployment facts.
