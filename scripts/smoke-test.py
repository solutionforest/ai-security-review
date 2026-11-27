"""Run the CLI with the synthetic fixture and a local model API stub."""

import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import subprocess
from threading import Thread


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = "fixtures/policy.txt"
MODEL = "synthetic-smoke-test"
CATEGORIES = {f"A{index:02}:2025" for index in range(1, 11)}


class MockModelAPI(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if (
            self.path != "/v1/chat/completions"
            or self.headers.get("Authorization") != "Bearer synthetic-token"
            or body.get("model") != MODEL
            or body.get("response_format") != {"type": "json_object"}
            or "tools" in body
            or "OWASP Web Application Top 10:2025" not in body["messages"][0]["content"]
            or list(json.loads(body["messages"][1]["content"])) != [FIXTURE]
        ):
            self.send_error(400, "Unexpected synthetic review request")
            return

        report = {"coverage": dict.fromkeys(sorted(CATEGORIES), "reviewed"), "findings": []}
        payload = json.dumps({
            "choices": [{"finish_reason": "stop", "message": {"content": json.dumps(report)}}]
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("binary", nargs="?", type=Path)
    group.add_argument("--image")
    args = parser.parse_args()
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()

    with HTTPServer(("127.0.0.1", 0), MockModelAPI) as server:
        settings = {
            "AI_REVIEW_API_KEY": "synthetic-token",
            "AI_REVIEW_BASE_URL": f"http://127.0.0.1:{server.server_port}/v1",
            "AI_REVIEW_MODEL": MODEL,
        }
        worker = Thread(target=server.serve_forever)
        worker.start()
        try:
            if args.image:
                command = [
                    "docker", "run", "--rm", "--network", "host",
                    "--user", f"{os.getuid()}:{os.getgid()}",
                    "--volume", f"{ROOT}:/work:ro",
                ]
                for key in settings:
                    command.extend(["--env", key])
                command.append(args.image)
            else:
                command = [str(args.binary.resolve())]

            result = subprocess.run(
                command + [".", sha, FIXTURE],
                cwd=ROOT,
                env={**os.environ, **settings},
                capture_output=True,
                text=True,
                timeout=45,
            )
        finally:
            server.shutdown()
            worker.join()

    if result.returncode:
        raise SystemExit(result.stderr.strip() or f"Reviewer exited {result.returncode}")
    report = json.loads(result.stdout)
    if (
        report["model"] != MODEL
        or report["reviewed_commit"] != sha
        or report["reviewed_paths"] != [FIXTURE]
        or set(report["coverage"]) != CATEGORIES
        or report["findings"] != []
    ):
        raise SystemExit("Unexpected synthetic review report")
    print("Synthetic review passed")


if __name__ == "__main__":
    main()
