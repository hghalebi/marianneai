# Delivery Plan

## 12:00 - 14:00 | MVP Build

### Wahid

- Build the open-data response service in `services/open-data-service`.
- Define how the service receives a normalized request and returns a generated answer.
- Support official French data sources and explicit source citation in every answer draft.
- Lock a first-pass contract with the email and web teams in `shared/contracts`.

### Michel

- Build the web interface in `services/web-interface`.
- Translate the MarianneAI product description into the landing page and demo interface.
- Use the shared request and response contract from `shared/contracts`.
- Ensure the UI can submit a request and render the service response clearly.

### Hamze / Team

- Build the email interface integration in `services/email-interface`.
- Parse inbound email into the shared request format.
- Preserve human review before any answer is sent back through email.
- Send generated answers back through email using the same response contract as the web flow.

## 14:00 - 16:00 | Productionize

- Stabilize the request-to-response pipeline across all services.
- Improve prompts and retrieval logic through `shared/prompts`.
- Expand the catalog of official French data sources through `shared/data-sources`.
- Connect the email flow and web flow to the same open-data response service.
- Prepare demo scenarios in `shared/demo-scenarios`.

## 16:00 - 16:45 | Demo Preparation

- Record a short video demo.
- Test the end-to-end email path and web path.
- Validate shared demo scenarios.
- Submit the project.

## Exit Criteria

- One service directory per owner with isolated implementation work.
- One shared contract for cross-service communication.
- One shared source catalog covering official French datasets used in the demo.
- One open-data answer path reused by email and web.
- One demo checklist covering the full user journey.
