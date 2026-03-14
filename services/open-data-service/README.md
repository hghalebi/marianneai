# Open Data Service

Owner: Wahid

## Responsibility

- Retrieve relevant official French public data
- Build prompt context from retrieved data
- Generate the response returned to email and web clients
- Attach explicit source citations and verification timestamps

## Inputs

- Normalized request payload from `shared/contracts`

## Outputs

- Shared response payload consumed by the email and web services

## Boundaries

- Do not implement email transport here
- Do not implement browser-specific UI here
- Keep prompt assets in `shared/prompts` when they are reused across flows
- Restrict retrieval to approved datasets listed in `shared/data-sources`
