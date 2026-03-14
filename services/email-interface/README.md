# Email Interface

Owner: Hamze / Team

## Responsibility

- Read inbound Gmail messages through `gws`
- Classify which emails are actually about MarianneAI
- Extract sender intent into queue-ready task candidates
- Decide whether each email is clear enough to become a queue item
- Persist valid tasks to Postgres on GCP

## Architecture

This service uses the supervisor-worker flow from
`industrial_doc_analyzer`:

1. `supervisor`
2. `mailbox_reader`
3. `relevance_classifier`
4. `intent_extractor`
5. `task_validator`
6. `queue_writer`
7. `answer_responder`
8. `datagouv_responder` (optional)
9. `auditor`

The shared blackboard lives in [`src/types.rs`](/Users/hamzeghalebi/projects/publicgood/marianneai/services/email-interface/src/types.rs), the orchestrator and specialists live in [`src/agentic.rs`](/Users/hamzeghalebi/projects/publicgood/marianneai/services/email-interface/src/agentic.rs), Gmail access lives in [`src/gmail.rs`](/Users/hamzeghalebi/projects/publicgood/marianneai/services/email-interface/src/gmail.rs), and database persistence lives in [`src/queue.rs`](/Users/hamzeghalebi/projects/publicgood/marianneai/services/email-interface/src/queue.rs).

## Inputs

- Gmail messages loaded via the Google Workspace CLI
- MarianneAI project profile:
  `L’IA citoyenne pour comprendre les données publiques.`

## Outputs

- Final mission report JSON in `output/email-triage/final_report_*.json`
- One-line mission summary in `output/email-triage/triage_summary.txt`
- Step-by-step context snapshots for debugging (`step_0000_context.json`, ...)
- Queue records in Postgres when a request is clear enough
- Sent Gmail replies when an answer endpoint is configured and response sending succeeds
- Sent Gmail replies from Datagouv MCP when answer endpoint is not configured and
  `DATAGOUV_MCP_ENDPOINT` is set

## Required Setup

### Environment

```bash
export GEMINI_API_KEY="..."
export GEMINI_MODEL="gemini-2.5-flash" # optional
export GEMINI_FALLBACK_MODEL="gemini-1.5-flash" # optional
export USE_GEMINI_FALLBACK="false" # optional
```

`GEMINI_MODEL` defaults to `gemini-2.5-flash` when unset.
`GEMINI_FALLBACK_MODEL` defaults to `gemini-1.5-flash` when unset.
Enable `USE_GEMINI_FALLBACK=true` when you want to force the fallback model for high-volume runs (typically to reduce rate-limit pressure).

### Optional answer generation endpoint

If `ANSWER_ENDPOINT_URL` is configured, the service will build an answer instruction from triage results, call that endpoint, and send the returned text back through Gmail.

```bash
export ANSWER_ENDPOINT_URL="https://your-service.example.com/answer"
export ANSWER_ENDPOINT_API_KEY="..." # optional
export ANSWER_ENDPOINT_TIMEOUT_SECONDS="30" # optional
```

If `ANSWER_ENDPOINT_URL` is unset, the pipeline still runs triage and queue persistence. If
`DATAGOUV_MCP_ENDPOINT` is configured, the Datagouv MCP responder is used to draft replies.
Otherwise, the pipeline sends the temporary busy fallback message.

The service performs startup checks and prints explicit hints when Gmail access is not fully configured.
It validates:

- Google Workspace CLI command availability (`gws` or `GWS_BIN` / `GMAIL_CLI_COMMAND`)
- OAuth client environment variables or `~/.config/gws/client_secret.json`
- presence of Gmail auth cache directory

Hints printed at startup include the exact `gws auth login --scopes ...` command.

### Optional Datagouv MCP source

```bash
export DATAGOUV_MCP_ENDPOINT="https://mcp.data.gouv.fr/mcp"
export DATAGOUV_MCP_TOOL="search_datasets" # optional
export DATAGOUV_MCP_TIMEOUT_SECONDS="30" # optional
```

The configured endpoint must support Streamable MCP (it returns `text/event-stream` and accepts
`application/json, text/event-stream`). The code sets this automatically, but some public MCP gateways
still return `406` when an unsupported transport is used.

You can also run the French Open Data MCP locally and point the service at it:

```bash
cd /tmp
git clone https://github.com/datagouv/datagouv-mcp.git
cd datagouv-mcp
MCP_HOST=0.0.0.0 docker compose up -d
```

Then use:

```bash
export DATAGOUV_MCP_ENDPOINT="http://127.0.0.1:8000/mcp"
export DATAGOUV_MCP_TOOL="search_datasets"
```

If you do not use Docker, use `uv`:

```bash
cp .env.example .env
MCP_HOST=0.0.0.0 DATAGOUV_API_ENV=prod LOG_LEVEL=INFO uv run main.py
```

Quick local checks after start:

```bash
curl -i http://127.0.0.1:8000/health
curl -i -H "Accept: application/json, text/event-stream" \
     -H "Content-Type: application/json" \
     -H "Origin: http://127.0.0.1:8000" \
     -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{"roots":{"listChanged":false},"sampling":{}},"clientInfo":{"name":"email-interface","version":"0.1.0"}}}' \
     http://127.0.0.1:8000/mcp
```

For local `./.env` loading, use:

```bash
set -a
source .env
set +a
```

When `ANSWER_ENDPOINT_URL` is configured, the Datagouv path is skipped by default.

### Gmail access (`gws`) and OAuth ownership

`gws` reads OAuth credentials from its local config (typically `~/.config/gws`).
If the Google consent screen shows another developer email, you are still using
cached OAuth material for a different app. Replace it with your own app credentials first.

1. Create an OAuth client in your Google Cloud project and copy:
   - `CLIENT_ID`
   - `CLIENT_SECRET`
2. Configure them for your shell before first login (or write them to the active
   `gws` config file), for example:

```bash
export GOOGLE_WORKSPACE_CLI_CLIENT_ID="your-oauth-client-id"
export GOOGLE_WORKSPACE_CLI_CLIENT_SECRET="your-oauth-client-secret"
```

3. Re-authorize Gmail:

```bash
gws auth login --scopes https://www.googleapis.com/auth/gmail.modify,https://www.googleapis.com/auth/gmail.send
```

If scope errors appear, rerun the same command with full scope.

To clear stale app registration state, remove the cached `gws` auth artifacts and re-run login:

```bash
rm -rf ~/.config/gws
```

Then repeat the `gws auth login` step.

You can force the CLI binary name if needed:

```bash
export GMAIL_CLI_COMMAND="gws" # or "gwc" if you use that binary
```

## Running

```bash
cargo run -- --query "newer_than:14d" --max-emails 10
```

Examples:

```bash
cargo run -- --query "newer_than:7d" --max-emails 5
cargo run -- --query "subject:MarianneAI OR marianneai" --max-emails 20 --queue-backend none
```

This writes snapshots and the final report under `./output/email-triage` by default.

Model calls are synchronous per email and can take extra time on long prompts.
If it appears slow, start with a smaller `--max-emails`.

### Continuous loop (every minute)

Use `scripts/run_email_interface_loop.sh` to run continuously.

```bash
chmod +x scripts/run_email_interface_loop.sh
./scripts/run_email_interface_loop.sh
```

The loop reads configuration from environment variables (and optional `./.env`):

```bash
export QUERY="newer_than:7d"
export MAX_EMAILS="5"
export QUEUE_BACKEND="none"
export INTERVAL_SECONDS="60"
export OUTPUT_DIR="./output/email-triage"
export LOG_FILE="./output/email-loop.log"
export ENV_FILE="./.env"
```

Press `Ctrl+C` to stop the loop.

### Verbose runtime logs

Each run prints step-by-step logs to terminal:

- current supervisor/agent
- active email index/from/subject
- classification and intent decisions
- response generation result (including fallback reason)
- sent reply previews (sanitized)

## CLI Surface

```bash
cargo run -- --query "newer_than:14d" --max-emails 10 --output-dir ./output/email-triage --queue-backend none
```

Supported flags:

- `--query` for the Gmail search query
- `--max-emails` for the bounded mailbox read size
- `--output-dir` for snapshots and final reports
- `--queue-backend` with `none` or `postgres`
- `DATAGOUV_MCP_ENDPOINT` for optional Datagouv answer sourcing
- `DATAGOUV_MCP_TOOL` to choose the MCP tool name
- `DATAGOUV_MCP_TIMEOUT_SECONDS` for MCP timeout

## Queue Backends

### Postgres

```bash
export QUEUE_BACKEND=postgres
export POSTGRES_DSN="postgres://USER:PASSWORD@HOST:5432/DATABASE"
export POSTGRES_QUEUE_TABLE="job_queue" # optional
```

The service creates or updates the queue table automatically if it does not exist.

`ON CONFLICT (source_message_id)` keeps processing idempotent for reruns.

## Integration Contracts

### Queue payload

When a message is clear enough to queue, the service persists a payload containing:

- Message metadata such as Gmail message id, thread id, sender, and subject
- The relevance classification result
- The extracted intent
- The task assessment and priority

### Answer endpoint request

When `ANSWER_ENDPOINT_URL` is set, the service sends a structured instruction containing:

- Reply subject
- Project name, about text, and website
- Sender and original subject
- Message excerpt
- Classification, intent, and validation objects
- Response guidelines for the downstream generator

### Answer endpoint response

The endpoint may return:

- Plain text
- JSON with one of `answer`, `response`, `message`, `text`, or `body`

## Quality Gates

Before considering changes complete, run:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Boundaries

- Do not own the browser experience here
- Do not duplicate open-data retrieval or answer-generation logic
- Keep email-specific parsing, classification, and queue insertion isolated here
- Preserve human review when a request is ambiguous or not queue-ready
