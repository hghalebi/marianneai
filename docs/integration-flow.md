# Integration Flow

## Service roles

- `services/open-data-service` is the shared backend API. It receives a normalized user question, retrieves official data, and returns one grounded response payload.
- `services/web-interface` sends browser-originated questions to `open-data-service` and renders the returned answer, sources, and limitations.
- `services/email-interface` extracts a question from inbound email, calls `open-data-service` with the same payload shape, and formats the response for human-reviewed sending.

## Canonical contract

- Shared API contract: `shared/contracts/query-api-contract.json`
- Shared demo scenarios: `shared/demo-scenarios/scenarios.json`
- Shared prompts: `shared/prompts/*.txt`

All services should treat the contract in `shared/contracts` as the source of truth. The web and email services should not duplicate retrieval logic or prompt logic.

## Request flow

1. A user asks a question in the web interface or by email.
2. The entry service converts that user input into:
   - `POST /query`
   - Body: `{"query": "<natural language question>"}`
3. `open-data-service` runs the 3-agent workflow:
   - Orchestrator: rewrites the user question into 1 to 3 search queries
   - Dataset Scout: filters MCP/data.gouv results and selects up to 3 sources
   - Answer Synthesizer: writes a concise answer grounded only in selected sources
4. `open-data-service` returns a stable JSON response:
   - `user_query`
   - `selected_sources`
   - `answer`
   - `limitations`
   - `trace`
5. The caller renders or reformats that response without changing its meaning.

## Runtime modes

- `USE_MOCK_GEMINI=true`: local deterministic Gemini fallback
- `USE_MOCK_MCP=true`: local deterministic data.gouv/MCP fallback

These flags let frontend and email teams integrate immediately, even before real credentials or MCP wiring are available.

## Web integration notes

- Load suggestions from `GET /demo/scenarios`
- Send the selected or typed query to `POST /query`
- Render:
  - `answer` as the main response block
  - `selected_sources` as clickable official sources
  - `limitations` as caveats
  - `trace` optionally as loading/provenance steps

## Email integration notes

- Extract the citizen question from the inbound email
- Call `POST /query` with the same payload used by the web interface
- Convert the JSON response into an email draft
- Keep the human review step before sending

## MCP integration boundary

Real MCP wiring is isolated in `services/open-data-service/app/services/mcp_service.py`.

- Expected request from `open-data-service` to MCP:
  - `POST {MCP_SERVER_URL}{MCP_SEARCH_PATH}`
  - Body: `{"queries": ["query one", "query two"]}`
- Expected MCP response:
  - `{"results": [{"title": "...", "url": "...", "description": "..."}]}`

If MCP is unavailable or returns invalid data, `open-data-service` falls back to mock datasets to preserve demo reliability.
