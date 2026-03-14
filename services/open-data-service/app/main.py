from fastapi import FastAPI, Request, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse

from app.config import settings
from app.routes import demo, health, query, reports
from app.utils.logger import logger

app = FastAPI(
    title=settings.app_name,
    description="Backend API for grounded answers using Gemini agents and data.gouv retrieval.",
    version="0.1.0",
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=settings.cors_origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(health.router)
app.include_router(demo.router)
app.include_router(query.router)
app.include_router(reports.router)


@app.get("/", tags=["System"])
async def root() -> dict[str, object]:
    return {
        "service": settings.app_name,
        "status": "ok",
        "docs_url": "/docs",
        "endpoints": ["/", "/health", "/demo/scenarios", "/query", "/reports/{report_id}/{filename}"],
    }


@app.exception_handler(Exception)
async def unhandled_exception_handler(_: Request, exc: Exception) -> JSONResponse:
    logger.error("Unhandled application error: %s", exc, exc_info=True)
    return JSONResponse(
        status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
        content={"detail": "Internal server error."},
    )


@app.on_event("startup")
async def startup_event() -> None:
    logger.info(
        "Starting %s in %s mode on port %s",
        settings.app_name,
        settings.app_env,
        settings.app_port,
    )


@app.on_event("shutdown")
async def shutdown_event() -> None:
    logger.info("Shutting down %s", settings.app_name)
