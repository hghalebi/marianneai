from fastapi import APIRouter, HTTPException
from fastapi.responses import FileResponse

from app.services.report_service import ReportService

router = APIRouter()
report_service = ReportService()
CONTENT_TYPES = {
    ".pdf": "application/pdf",
    ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
}


@router.get("/reports/{report_id}/{filename}", tags=["Reports"])
async def download_report(report_id: str, filename: str) -> FileResponse:
    try:
        path = report_service.resolve_report_path(report_id, filename)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail="Invalid report path.") from exc

    if not path.exists():
        raise HTTPException(status_code=404, detail="Report not found.")

    return FileResponse(path, media_type=CONTENT_TYPES.get(path.suffix.lower(), "application/octet-stream"))
