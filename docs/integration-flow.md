# Integration Flow

## Service Roles

- `services/email-interface` handles inbound email, outbound email, and email-specific orchestration.
- `services/web-interface` handles the public-facing site, demo entry points, and user interaction in the browser.
- `services/open-data-service` handles retrieval from official French public data sources and response generation.

## Shared Request Path

1. A user submits a question through email or the web interface.
2. The entry service converts that input into the shared request format defined in `shared/contracts`.
3. The request is sent to `services/open-data-service`.
4. The open-data service retrieves data from approved sources in `shared/data-sources`, builds the prompt, and generates an answer draft.
5. The answer is returned in the shared response format with citations and verification metadata.
6. The email interface sends the reply by email after review, and the web interface renders the same response in the browser.

## Product Workflow

1. Une question arrive
2. MarianneAI cherche les bonnes sources
3. L'assistant genere une reponse claire
4. L'utilisateur verifie et envoie

## Contract Boundaries

- Email-specific metadata stays inside `services/email-interface` unless it is needed downstream.
- UI state stays inside `services/web-interface`.
- Retrieval logic, prompt assembly, source ranking, and answer generation stay inside `services/open-data-service`.
- Only stable cross-service payloads should move into `shared/contracts`.

## Demo Goal

The same user question should produce a comparable sourced answer whether it comes from email or the web interface. That shared behavior is the center of the demo.
