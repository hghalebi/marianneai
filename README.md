# MarianneAI

Monorepo layout for parallel MVP delivery of a French public-interest assistant that turns official open data into clear, sourced answers for citizens and public agents.

## Repository Layout

```text
.
|-- docs/
|   |-- demo-checklist.md
|   |-- integration-flow.md
|   `-- plan.md
|-- infra/
|   `-- README.md
|-- services/
|   |-- email-interface/
|   |   |-- README.md
|   |   |-- src/
|   |   `-- tests/
|   |-- open-data-service/
|   |   |-- README.md
|   |   |-- src/
|   |   `-- tests/
|   `-- web-interface/
|       |-- content/
|       |-- README.md
|       |-- src/
|       `-- tests/
`-- shared/
    |-- contracts/
    |   `-- README.md
    |-- data-sources/
    |   `-- README.md
    |-- demo-scenarios/
    |   `-- README.md
    `-- prompts/
        `-- README.md
```

## Product Principles

- Sources officielles uniquement
- Reponses claires, verifiables et sourcees
- Transparence totale sur l'origine des chiffres
- Validation humaine avant envoi

## Ownership

- Wahid owns [`services/open-data-service`](./services/open-data-service/README.md) and is responsible for retrieval from official French data sources plus answer generation.
- Michel owns [`services/web-interface`](./services/web-interface/README.md) and is responsible for the landing page, demo experience, and agent-facing web flow.
- Hamze and team own [`services/email-interface`](./services/email-interface/README.md) and are responsible for inbound email, outbound replies, and human-reviewed sending.
- Shared integration work belongs in [`shared/contracts`](./shared/contracts/README.md), [`shared/data-sources`](./shared/data-sources/README.md), [`shared/prompts`](./shared/prompts/README.md), and [`docs/integration-flow.md`](./docs/integration-flow.md).

## Working Rules

- Keep implementation local to your service unless the change is an agreed integration contract.
- Treat `shared/contracts` as the source of truth for request and response payloads between services.
- Reuse the same response-generation contract from both email and web flows so demo behavior stays consistent.
- Keep source citation and verification metadata attached to every answer draft.
- Do not design for blind auto-send; the human reviewer stays in the loop.
- Track delivery against [`docs/plan.md`](./docs/plan.md) and rehearse from [`docs/demo-checklist.md`](./docs/demo-checklist.md).

## Immediate Goal

The first milestone is a connected MVP where:

1. The email interface receives a request and normalizes it.
2. The open-data service retrieves relevant official French public data and generates a sourced response.
3. The web interface can submit the same request shape and display the same answer format.
4. A human can verify the answer draft and sources before sending.
5. The team can demo both entry points with shared scenarios.
