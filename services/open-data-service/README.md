# Open Data Service

Owner: Wahid

FastAPI backend for `DataGouv Alive`. This service exposes a stable query API for the web and email interfaces, orchestrates Gemini-based agents, and retrieves official data.gouv.fr sources through an MCP adapter.

## Responsibilities

- Receive normalized requests from the web and email services
- Use a lightweight 3-agent orchestration flow
- Retrieve relevant official datasets through a real or mocked MCP adapter
- Return grounded answers with selected sources, limitations, execution trace, and structured analytics

## Shared integration points

- Request/response contract: [`shared/contracts/query-api-contract.json`](../../shared/contracts/query-api-contract.json)
- Shared prompts: [`shared/prompts`](../../shared/prompts)
- Shared demo scenarios: [`shared/demo-scenarios/scenarios.json`](../../shared/demo-scenarios/scenarios.json)
- Cross-service flow: [`docs/integration-flow.md`](../../docs/integration-flow.md)

## Local setup

```bash
cd services/open-data-service
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
cp .env.example .env
uvicorn app.main:app --reload --port 8000
```

## Endpoints

- `GET /health`
- `GET /demo/scenarios`
- `POST /query`
- `GET /reports/{report_id}/{filename}`

## Mock mode

The service is demo-safe by default:

- `USE_MOCK_GEMINI=true` avoids live Gemini calls
- `USE_MOCK_MCP=false` uses the official hosted data.gouv MCP server by default
- Set `USE_MOCK_MCP=true` only if you need an offline demo fallback

With both flags enabled, the API still returns stable JSON and scenario-dependent sample sources.

## Full analysis mode

The backend now supports two analytics layers:

- Full-resource local analytics: if a CSV or JSON resource is small enough, the service downloads it completely and computes:
  - descriptive statistics
  - linear regressions
  - frontend-ready chart payloads for Recharts
- Vertex code execution analytics: optional mode using Vertex AI code execution through the `google-genai` SDK

The API response includes:

- `analysis_engine`
- `dataset_row_count`
- `dataset_columns`
- `descriptive_statistics`
- `regressions`
- `charts`
- `report_artifacts`

Relevant environment variables:

- `ENABLE_FULL_RESOURCE_DOWNLOAD=true`
- `MAX_FULL_RESOURCE_BYTES=8000000`
- `MAX_FULL_RESOURCE_ROWS=25000`
- `ENABLE_VERTEX_CODE_EXECUTION=false`
- `GOOGLE_CLOUD_PROJECT=...`
- `GOOGLE_CLOUD_LOCATION=europe-west1`
- `VERTEX_CODE_EXECUTION_MODEL=gemini-2.5-flash`

If Vertex code execution is disabled or unavailable, the backend falls back to deterministic local analytics.

## Real MCP integration

The adapter lives in `app/services/mcp_service.py`.

- Default endpoint: `https://mcp.data.gouv.fr/mcp`
- The service uses the official MCP Python SDK and Streamable HTTP transport
- It calls the official tools exposed by data.gouv:
  - `search_datasets`
  - `get_dataset_info`
  - `list_dataset_resources`
  - `get_resource_info`
- If the live MCP call fails, the backend falls back to deterministic mock data for demo resilience

## Real Gemini integration

The wrapper lives in `app/services/gemini_service.py`.

- Set `GEMINI_API_KEY`
- Set `USE_MOCK_GEMINI=false`
- Adjust `GEMINI_MODEL` if needed

If Gemini or MCP fails at runtime, the service degrades to deterministic mock behavior for demo reliability.

## Vertex code execution

The optional analytics backend lives in `app/services/code_interpreter_analytics.py`.

- It accepts a raw CSV/JSON buffer produced after MCP retrieval
- It can use Vertex AI code execution when enabled
- It returns structured JSON that the frontend can render directly
- It falls back to local deterministic analytics when Vertex is unavailable
