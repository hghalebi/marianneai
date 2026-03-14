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
