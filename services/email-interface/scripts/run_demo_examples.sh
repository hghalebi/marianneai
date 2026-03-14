#!/usr/bin/env bash

set -euo pipefail

# Run several demo commands in a single invocation.
# Each demo is independent (its own output directory and labeled log lines).

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly ENV_FILE="${ENV_FILE:-${REPO_DIR}/.env}"
readonly DEMO_OUTPUT_DIR="${DEMO_OUTPUT_DIR:-${REPO_DIR}/output/demo}"
readonly DEMO_MAX_EMAILS="${DEMO_MAX_EMAILS:-1}"
readonly DEMO_QUEUE_BACKEND="${DEMO_QUEUE_BACKEND:-none}"

mkdir -p "${DEMO_OUTPUT_DIR}"

if [[ -f "${ENV_FILE}" ]]; then
    set -o allexport
    # shellcheck disable=SC1090
    source "${ENV_FILE}"
    set +o allexport
fi

run_example() {
    local title="$1"
    local query="$2"
    local max_emails="${3:-${DEMO_MAX_EMAILS}}"
    local safe_title
    local output_dir

    safe_title="$(printf "%s" "${title}" | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-')"
    output_dir="${DEMO_OUTPUT_DIR}/${safe_title}"

    printf '\n\033[1m== Demo: %s ==\033[0m\n' "${title}"
    printf 'Query: %s\nMax emails: %s\nOutput: %s\n' "${query}" "${max_emails}" "${output_dir}"

    mkdir -p "${output_dir}"

    (cd "${REPO_DIR}" && cargo run -- --query "${query}" --max-emails "${max_emails}" --output-dir "${output_dir}" --queue-backend "${DEMO_QUEUE_BACKEND}")
}

run_example "marianneai subject mailbox" "subject:marianneai" 1
run_example "marianneai recent issues" "subject:marianneai newer_than:7d" 2
run_example "marianneai public data lookup" "marianneai public data companies france" 2
run_example "generic marianneai" "marianneai" 2

printf '\nDemo examples completed. Logs and reports are under %s\n' "${DEMO_OUTPUT_DIR}"
