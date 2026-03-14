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
    analysis_engine: str = "heuristic-local"
    analysis_summary: str = ""
    key_findings: List[str] = Field(default_factory=list)
    data_coverage: str = ""
    dataset_row_count: int | None = None
    dataset_columns: List[str] = Field(default_factory=list)
    descriptive_statistics: List["DescriptiveStatistic"] = Field(default_factory=list)
    regressions: List["RegressionResult"] = Field(default_factory=list)
    charts: List["AnalyticsChart"] = Field(default_factory=list)
    used_resources: List["UsedResource"] = Field(default_factory=list)
    report_artifacts: List["ReportArtifact"] = Field(default_factory=list)


class DemoScenariosResponse(BaseModel):
    scenarios: List[str]


class HealthResponse(BaseModel):
    status: str
    service: str
    environment: str
    mock_gemini: bool
    mock_mcp: bool
    vertex_code_execution_enabled: bool


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


class MCPResourceRecord(BaseModel):
    id: str = ""
    title: str
    url: HttpUrl
    format: str = ""
    description: str = ""
    last_modified: str = ""
    metadata: Dict[str, Any] = Field(default_factory=dict)


class MCPDatasetDetails(MCPDatasetRecord):
    resources: List[MCPResourceRecord] = Field(default_factory=list)


class UsedResource(BaseModel):
    dataset_title: str
    resource_title: str
    resource_url: HttpUrl
    format: str = ""


class AnalysisResult(BaseModel):
    analysis_engine: str = "heuristic-local"
    analysis_summary: str = ""
    key_findings: List[str] = Field(default_factory=list)
    data_coverage: str = ""
    dataset_row_count: int | None = None
    dataset_columns: List[str] = Field(default_factory=list)
    descriptive_statistics: List["DescriptiveStatistic"] = Field(default_factory=list)
    regressions: List["RegressionResult"] = Field(default_factory=list)
    charts: List["AnalyticsChart"] = Field(default_factory=list)
    used_resources: List[UsedResource] = Field(default_factory=list)
    limitations: List[str] = Field(default_factory=list)
    resource_analyses: List["ResourceAnalysis"] = Field(default_factory=list)


class ResourceSample(BaseModel):
    rows: List[Dict[str, Any]] = Field(default_factory=list)
    total_rows: int | None = None
    columns: List[str] = Field(default_factory=list)
    row_count_sampled: int = 0


class ResourceTable(BaseModel):
    text: str = ""
    rows: List[Dict[str, Any]] = Field(default_factory=list)
    total_rows: int | None = None
    columns: List[str] = Field(default_factory=list)
    row_count_analyzed: int = 0
    content_bytes: int = 0
    full_download_used: bool = False


class DescriptiveStatistic(BaseModel):
    column: str
    non_null_count: int = 0
    mean: float | None = None
    min: float | None = None
    max: float | None = None
    median: float | None = None
    stddev: float | None = None


class RegressionResult(BaseModel):
    feature_x: str
    feature_y: str
    slope: float
    intercept: float
    r_squared: float
    sample_size: int


class AnalyticsChart(BaseModel):
    chart_id: str
    title: str
    chart_type: str
    description: str = ""
    x_key: str
    y_keys: List[str] = Field(default_factory=list)
    data: List[Dict[str, Any]] = Field(default_factory=list)


class AnalyticsComputation(BaseModel):
    analysis_engine: str = "heuristic-local"
    analysis_summary: str = ""
    key_findings: List[str] = Field(default_factory=list)
    data_coverage: str = ""
    row_count: int | None = None
    columns: List[str] = Field(default_factory=list)
    descriptive_statistics: List[DescriptiveStatistic] = Field(default_factory=list)
    regressions: List[RegressionResult] = Field(default_factory=list)
    charts: List[AnalyticsChart] = Field(default_factory=list)
    limitations: List[str] = Field(default_factory=list)


class ResourceAnalysis(BaseModel):
    dataset_id: str = ""
    dataset_title: str
    resource_id: str = ""
    resource_title: str
    resource_url: HttpUrl
    format: str = ""
    score: int = 0
    coverage: str = ""
    sample: ResourceSample = Field(default_factory=ResourceSample)
    table: ResourceTable = Field(default_factory=ResourceTable)
    analysis_engine: str = "heuristic-local"
    descriptive_statistics: List[DescriptiveStatistic] = Field(default_factory=list)
    regressions: List[RegressionResult] = Field(default_factory=list)
    charts: List[AnalyticsChart] = Field(default_factory=list)
    findings: List[str] = Field(default_factory=list)


class ReportArtifact(BaseModel):
    report_id: str
    format: str
    filename: str
    download_url: str
    content_type: str
