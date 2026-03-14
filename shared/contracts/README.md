# Shared Contracts

Use this directory for the payloads and interface definitions shared across services.

Recommended contents:

- Request schema used by email and web when calling the open-data service
- Response schema returned by the open-data service
- Example payloads for integration testing

## Minimum Shared Fields

- User question
- Request channel such as `email` or `web`
- Geography or administrative scope when available
- Time period when available
- Generated answer draft
- Source list with title, publisher, link, and retrieval date
- Verification status indicating the answer still needs human review

## Rule

If a change affects more than one service, define or update the contract here first.
