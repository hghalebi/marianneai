from fastapi import APIRouter, HTTPException, status

from app.schemas import QueryRequest, QueryResponse
from app.services.orchestrator import QueryOrchestrator
from app.utils.logger import logger

router = APIRouter()
orchestrator = QueryOrchestrator()


@router.post("/query", response_model=QueryResponse, tags=["Core"])
async def process_query(request: QueryRequest) -> QueryResponse:
    try:
        return await orchestrator.process_query(request)
    except FileNotFoundError as exc:
        logger.error("Missing shared asset: %s", exc)
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="A shared prompt or scenario file is missing.",
        ) from exc
    except Exception as exc:
        logger.error("Unhandled error while processing query: %s", exc, exc_info=True)
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="An internal error occurred while processing the query.",
        ) from exc
