# Email Interface

Owner: Hamze / Team

## Responsibility

- Receive inbound email requests
- Normalize email content into the shared request format
- Support a human-reviewed response workflow before sending answers back through email

## Inputs

- Incoming email payloads
- Shared response payload from the open-data service

## Outputs

- Shared request payload sent to the open-data service
- Email replies sent back to the user

## Boundaries

- Do not own the browser experience here
- Do not duplicate open-data retrieval or prompt logic here
- Keep email-specific parsing and transport isolated to this service
- Preserve the review-and-send step described in the product flow
