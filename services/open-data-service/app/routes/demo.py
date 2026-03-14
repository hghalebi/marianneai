import json

from fastapi import APIRouter

from app.config import settings
from app.schemas import DemoScenariosResponse

router = APIRouter()


@router.get("/demo/scenarios", response_model=DemoScenariosResponse, tags=["Demo"])
async def get_scenarios() -> DemoScenariosResponse:
    scenarios_file = settings.shared_dir / "demo-scenarios" / "scenarios.json"
    if scenarios_file.exists():
        return DemoScenariosResponse.model_validate(json.loads(scenarios_file.read_text(encoding="utf-8")))
    return DemoScenariosResponse(
        scenarios=["What official datasets are available for electric vehicle charging stations in France?"]
    )
