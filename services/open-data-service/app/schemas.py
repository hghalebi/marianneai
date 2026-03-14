from typing import Any, Dict, List

from pydantic import BaseModel, Field, HttpUrl


class QueryRequest(BaseModel):
    query: str = Field(..., min_length=3, description="The user's natural language query")


class Source(BaseModel):
    title: str
    url: HttpUrl
    description: str
    reason_for_selection: str
    confidence_score: float = Field(..., ge=0.0, le=1.0)


class QueryResponse(BaseModel):
    user_query: str
    selected_sources: List[Source]
    answer: str
    limitations: List[str]
    trace: List[str]


class DemoScenariosResponse(BaseModel):
    scenarios: List[str]


class HealthResponse(BaseModel):
    status: str
    service: str
    environment: str
    mock_gemini: bool
    mock_mcp: bool


class OrchestratorPlan(BaseModel):
    search_queries: List[str] = Field(description="Search queries to send to data.gouv.fr")
    reasoning: str = Field(description="Why these queries were chosen")


class ScoutSelection(BaseModel):
    selected_sources: List[Source] = Field(description="Filtered and scored relevant sources")


class SynthesizedAnswer(BaseModel):
    answer: str = Field(description="Final synthesized answer")
    limitations: List[str] = Field(description="Known limitations and uncertainty")


class MCPDatasetRecord(BaseModel):
    id: str = ""
    title: str
    description: str = ""
    url: HttpUrl
    organization: str = ""
    metadata: Dict[str, Any] = Field(default_factory=dict)
