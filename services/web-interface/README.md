# Web Interface

Owner: Michel

## Responsibility

- Provide the public-facing MarianneAI web experience
- Capture user questions and show generated responses
- Host the landing page sections such as `Comment ca marche`, `Cas d'usage`, `Donnees sources`, `FAQ`, and `Demander une demo`
- Integrate with the shared request and response contract

## Inputs

- User input from the browser
- Shared response payload from the open-data service

## Outputs

- Shared request payload sent to the open-data service
- Browser-rendered response

## Boundaries

- Do not duplicate retrieval or answer-generation logic here
- Do not own outbound email behavior here
- Keep contract changes synchronized through `shared/contracts`

## Docker

- Build and run locally with `docker compose up --build` from `services/web-interface`
- The front is exposed on `http://localhost:4000`
- Set `BACKEND_URL` in your shell or a local `.env` file to connect the proxy to the open-data service
- Override `GEMINI_API_KEY` in your shell or a local `.env` file before starting the stack if needed
