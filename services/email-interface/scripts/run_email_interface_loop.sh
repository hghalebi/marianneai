#!/usr/bin/env bash

set -euo pipefail

# Run the email interface on a fixed interval. This keeps the binary hot and logs
# each run for troubleshooting.

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly LOCK_DIR="${REPO_DIR}/.run-loop.lock"
readonly LOG_FILE="${LOG_FILE:-${REPO_DIR}/output/email-loop.log}"
readonly INTERVAL_SECONDS="${INTERVAL_SECONDS:-60}"
readonly QUERY="${QUERY:-newer_than:7d}"
readonly MAX_EMAILS="${MAX_EMAILS:-10}"
readonly OUTPUT_DIR="${OUTPUT_DIR:-./output/email-triage}"
readonly QUEUE_BACKEND="${QUEUE_BACKEND:-none}"
readonly ENV_FILE="${ENV_FILE:-${REPO_DIR}/.env}"

mkdir -p "${REPO_DIR}/output"

if [[ -f "${ENV_FILE}" ]]; then
    # shellcheck disable=SC1090
    set -o allexport
    source "${ENV_FILE}"
    set +o allexport
fi

log() {
    printf "[%s] %s\n" "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$1" >> "${LOG_FILE}"
}

run_once() {
    local start_ts
    local end_ts
    start_ts="$(date -u +%s)"

    log "Starting email interface run"
    ( cd "${REPO_DIR}" && cargo run \
        -- \
        --query "${QUERY}" \
        --max-emails "${MAX_EMAILS}" \
        --output-dir "${OUTPUT_DIR}" \
        --queue-backend "${QUEUE_BACKEND}" ) >> "${LOG_FILE}" 2>&1
    local status=$?
    end_ts="$(date -u +%s)"

    if (( status == 0 )); then
        log "Run completed in $((end_ts - start_ts))s"
    else
        log "Run failed with status ${status}"
    fi

    return "${status}"
}

# Prevent parallel scheduler overlap if two instances start accidentally.
if [[ -e "${LOCK_DIR}" ]] && [[ ! -d "${LOCK_DIR}" ]]; then
    rm -f "${LOCK_DIR}"
fi
if ! mkdir "${LOCK_DIR}" 2>/dev/null; then
    echo "Loop is already running. Exiting." >&2
    exit 1
fi
trap 'rmdir "${LOCK_DIR}"' EXIT

log "Starting loop with interval=${INTERVAL_SECONDS}s query=${QUERY} max_emails=${MAX_EMAILS} queue_backend=${QUEUE_BACKEND}"

while true; do
    run_once || true
    sleep "${INTERVAL_SECONDS}"
done
