# Open Data Service

Owner: Wahid

FastAPI backend for `DataGouv Alive`. This service exposes a stable query API for the web and email interfaces, orchestrates Gemini-based agents, and retrieves official data.gouv.fr sources through an MCP adapter.

## Responsibilities

- Receive normalized requests from the web and email services
- Use a lightweight 3-agent orchestration flow
- Retrieve relevant official datasets through a real or mocked MCP adapter
- Return grounded answers with selected sources, limitations, and execution trace

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

## Mock mode

The service is demo-safe by default:

- `USE_MOCK_GEMINI=true` avoids live Gemini calls
- `USE_MOCK_MCP=true` avoids live MCP/data.gouv calls

With both flags enabled, the API still returns stable JSON and scenario-dependent sample sources.

## Real MCP integration

The adapter lives in `app/services/mcp_service.py`.

- Update `MCP_SERVER_URL` and `MCP_SEARCH_PATH`
- Keep the payload format: `{"queries": ["..."]}`
- Return records under a `results` key
- Each result should include at least `title`, `url`, and `description`

## Real Gemini integration

The wrapper lives in `app/services/gemini_service.py`.

- Set `GEMINI_API_KEY`
- Set `USE_MOCK_GEMINI=false`
- Adjust `GEMINI_MODEL` if needed

If Gemini or MCP fails at runtime, the service degrades to deterministic mock behavior for demo reliability.
