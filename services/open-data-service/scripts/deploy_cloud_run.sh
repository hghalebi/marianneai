#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SERVICE_DIR="${ROOT_DIR}/services/open-data-service"

if ! command -v gcloud >/dev/null 2>&1; then
  echo "gcloud is required but not installed."
  exit 1
fi

PROJECT_ID="${GOOGLE_CLOUD_PROJECT:-$(gcloud config get-value project 2>/dev/null || true)}"
REGION="${GOOGLE_CLOUD_LOCATION:-europe-west1}"
SERVICE_NAME="${SERVICE_NAME:-open-data-service}"
REPOSITORY="${ARTIFACT_REPOSITORY:-marianneai}"
IMAGE_NAME="${IMAGE_NAME:-open-data-service}"
IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPOSITORY}/${IMAGE_NAME}:latest"
GEMINI_SECRET="${GEMINI_SECRET:-GEMINI_API_KEY}"

if [[ -z "${PROJECT_ID}" ]]; then
  echo "GOOGLE_CLOUD_PROJECT is empty. Set it in the environment or with 'gcloud config set project ...'."
  exit 1
fi

echo "Project: ${PROJECT_ID}"
echo "Region: ${REGION}"
echo "Service: ${SERVICE_NAME}"
echo "Image: ${IMAGE_URI}"

gcloud services enable \
  run.googleapis.com \
  cloudbuild.googleapis.com \
  artifactregistry.googleapis.com \
  secretmanager.googleapis.com

if ! gcloud artifacts repositories describe "${REPOSITORY}" --location "${REGION}" >/dev/null 2>&1; then
  gcloud artifacts repositories create "${REPOSITORY}" \
    --repository-format=docker \
    --location="${REGION}" \
    --description="MarianneAI Docker images"
fi

cd "${ROOT_DIR}"

gcloud builds submit \
  --config "${SERVICE_DIR}/cloudbuild.cloudrun.yaml" \
  --substitutions \
_SERVICE_NAME="${SERVICE_NAME}",\
_REGION="${REGION}",\
_IMAGE_URI="${IMAGE_URI}",\
_GEMINI_SECRET="${GEMINI_SECRET}",\
_USE_MOCK_GEMINI="${USE_MOCK_GEMINI:-false}",\
_USE_MOCK_MCP="${USE_MOCK_MCP:-false}",\
_ENABLE_FULL_RESOURCE_DOWNLOAD="${ENABLE_FULL_RESOURCE_DOWNLOAD:-true}",\
_MAX_FULL_RESOURCE_BYTES="${MAX_FULL_RESOURCE_BYTES:-8000000}",\
_MAX_FULL_RESOURCE_ROWS="${MAX_FULL_RESOURCE_ROWS:-25000}",\
_ENABLE_VERTEX_CODE_EXECUTION="${ENABLE_VERTEX_CODE_EXECUTION:-false}",\
_VERTEX_CODE_EXECUTION_MODEL="${VERTEX_CODE_EXECUTION_MODEL:-gemini-2.5-flash}",\
_MAX_VERTEX_BUFFER_CHARS="${MAX_VERTEX_BUFFER_CHARS:-200000}",\
_MCP_SERVER_URL="${MCP_SERVER_URL:-https://mcp.data.gouv.fr/mcp}",\
_CORS_ALLOW_ORIGINS="${CORS_ALLOW_ORIGINS:-*}" \
  .
