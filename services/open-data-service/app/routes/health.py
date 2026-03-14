from fastapi import APIRouter

from app.config import settings
from app.schemas import HealthResponse

router = APIRouter()


@router.get("/health", response_model=HealthResponse, tags=["System"])
async def health_check() -> HealthResponse:
    return HealthResponse(
        status="ok",
        service=settings.app_name,
        environment=settings.app_env,
        mock_gemini=settings.use_mock_gemini,
        mock_mcp=settings.use_mock_mcp,
        vertex_code_execution_enabled=settings.enable_vertex_code_execution,
    )
