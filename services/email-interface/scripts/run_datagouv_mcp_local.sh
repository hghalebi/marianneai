#!/usr/bin/env bash

set -euo pipefail

# Start/stop a local datagouv MCP server and (optionally) verify health.

readonly WORKDIR="${WORKDIR:-/tmp/datagouv-mcp}"
readonly MCP_REPO_URL="${MCP_REPO_URL:-https://github.com/datagouv/datagouv-mcp.git}"
readonly MCP_HOST_NAME="${DATAGOUV_MCP_HOST:-127.0.0.1}"
readonly MCP_BIND_HOST="${DATAGOUV_MCP_BIND_HOST:-0.0.0.0}"
readonly MCP_SERVER_PORT="${DATAGOUV_MCP_PORT:-8000}"
readonly MCP_API_ENV="${DATAGOUV_API_ENV:-prod}"
readonly MCP_LOG_LEVEL="${LOG_LEVEL:-INFO}"

readonly ACTION="${1:-up}"

readonly ENDPOINT="http://${MCP_HOST_NAME}:${MCP_SERVER_PORT}/mcp"
readonly HEALTH_URL="http://${MCP_HOST_NAME}:${MCP_SERVER_PORT}/health"

if ! command -v docker >/dev/null 2>&1; then
    echo "docker not found. Install Docker Desktop first." >&2
    exit 1
fi

if [[ "${ACTION}" == "down" ]]; then
    if [[ -d "${WORKDIR}" ]]; then
        (cd "${WORKDIR}" && docker compose down)
        echo "Stopped local MCP container set."
    else
        echo "No local MCP directory found at ${WORKDIR}. Nothing to stop."
    fi
    exit 0
fi

if [[ "${ACTION}" == "health" ]]; then
    echo "Checking local MCP health at ${HEALTH_URL}"
    curl -sSf "${HEALTH_URL}"
    echo
    exit 0
fi

if [[ "${ACTION}" == "up" ]] || [[ "${ACTION}" == "start" ]]; then
    if [[ ! -d "${WORKDIR}" ]]; then
        git clone "${MCP_REPO_URL}" "${WORKDIR}"
    fi
    (
        cd "${WORKDIR}"
        MCP_HOST="${MCP_BIND_HOST}" \
            MCP_PORT="${MCP_SERVER_PORT}" \
            DATAGOUV_API_ENV="${MCP_API_ENV}" \
            LOG_LEVEL="${MCP_LOG_LEVEL}" \
            docker compose up -d
    )

    echo "Started MCP container. Waiting for health endpoint ${HEALTH_URL}..."
    for _ in {1..30}; do
        if curl -sSf "${HEALTH_URL}" >/dev/null 2>&1; then
            echo "Local datagouv MCP is ready."
            echo "Datagouv MCP endpoint: ${ENDPOINT}"
            echo "Set DATAGOUV_MCP_ENDPOINT=${ENDPOINT} (or keep it unset for auto-detect)."
            exit 0
        fi
        sleep 1
    done
    echo "Timeout waiting for ${HEALTH_URL}. Check logs in ${WORKDIR}." >&2
    exit 1
fi

echo "Usage: ${0} [up|start|down|health]"
echo "Default action is up."
exit 1
