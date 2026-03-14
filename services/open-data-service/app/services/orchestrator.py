import json

from app.config import settings
from app.schemas import OrchestratorPlan, QueryRequest, QueryResponse, ScoutSelection, Source, SynthesizedAnswer
from app.services.gemini_service import GeminiService
from app.services.mcp_service import MCPService
from app.utils.logger import logger


class QueryOrchestrator:
    """Coordinates the Orchestrator, Dataset Scout, and Answer Synthesizer agents."""

    def __init__(self) -> None:
        self.gemini = GeminiService()
        self.mcp = MCPService()
        self.prompts_dir = settings.shared_dir / "prompts"

    def _load_prompt(self, filename: str, **kwargs: str) -> str:
        prompt_path = self.prompts_dir / filename
        if not prompt_path.exists():
            raise FileNotFoundError(f"Missing prompt file: {prompt_path}")
        return prompt_path.read_text(encoding="utf-8").format(**kwargs)

    async def process_query(self, request: QueryRequest) -> QueryResponse:
        logger.info("Processing query: %s", request.query)
        trace = ["Started query processing."]

        trace.append("Agent 1 (Orchestrator) analyzing query...")
        plan_prompt = self._load_prompt("orchestrator.txt", query=request.query)
        plan = self.gemini.generate_structured(
            plan_prompt,
            OrchestratorPlan,
            self._build_mock_plan(request.query),
        )
        trace.append(f"Orchestrator decided to search for: {plan.search_queries}")

        trace.append("Calling MCP data.gouv.fr tools...")
        raw_results = await self.mcp.search_data_gouv(plan.search_queries)
        trace.append(f"MCP returned {len(raw_results)} raw datasets.")

        trace.append("Agent 2 (Dataset Scout) filtering results...")
        scout_prompt = self._load_prompt(
            "dataset_scout.txt",
            query=request.query,
            raw_results=json.dumps(raw_results, ensure_ascii=False, indent=2),
        )
        selection = self.gemini.generate_structured(
            scout_prompt,
            ScoutSelection,
            self._build_mock_scout(request.query, raw_results),
        )
        trace.append(f"Scout selected {len(selection.selected_sources)} relevant sources.")

        trace.append("Agent 3 (Answer Synthesizer) generating final response...")
        synth_prompt = self._load_prompt(
            "answer_synthesizer.txt",
            query=request.query,
            sources=selection.model_dump_json(indent=2),
        )
        synthesis = self.gemini.generate_structured(
            synth_prompt,
            SynthesizedAnswer,
            self._build_mock_synthesis(request.query, selection.selected_sources),
        )
        trace.append("Synthesis complete.")

        return QueryResponse(
            user_query=request.query,
            selected_sources=selection.selected_sources,
            answer=synthesis.answer,
            limitations=synthesis.limitations,
            trace=trace,
        )

    def _build_mock_plan(self, query: str) -> OrchestratorPlan:
        lower_query = query.lower()
        if "eau" in lower_query or "water" in lower_query:
            return OrchestratorPlan(
                search_queries=["qualité eau potable paris", "eau potable Île-de-France"],
                reasoning="The query is about water quality, so the search should target public health and local monitoring datasets.",
            )
        if any(keyword in lower_query for keyword in ["transport", "lyon", "retard", "mobilité", "mobilite"]):
            return OrchestratorPlan(
                search_queries=["ponctualité TCL Lyon", "retards transport Lyon"],
                reasoning="The query is about public transport performance in Lyon, so the search focuses on punctuality and delays.",
            )
        return OrchestratorPlan(
            search_queries=["bornes recharge véhicules électriques", "IRVE France"],
            reasoning="The query is about electric vehicle charging infrastructure in France.",
        )

    def _build_mock_scout(self, query: str, raw_results: list[dict]) -> ScoutSelection:
        selected_sources = []
        for index, item in enumerate(raw_results[:3]):
            selected_sources.append(
                Source(
                    title=item["title"],
                    url=item["url"],
                    description=item.get("description", ""),
                    reason_for_selection=self._selection_reason(query, index),
                    confidence_score=max(0.7, round(0.95 - index * 0.1, 2)),
                )
            )
        return ScoutSelection(selected_sources=selected_sources)

    def _build_mock_synthesis(self, query: str, sources: list[Source]) -> SynthesizedAnswer:
        if not sources:
            return SynthesizedAnswer(
                answer="I could not identify a reliable official source for this query from the retrieved results.",
                limitations=[
                    "No retrieved source was relevant enough to support a grounded answer.",
                    "The workflow is intentionally restricted to retrieved official data.gouv.fr sources.",
                ],
            )

        primary = sources[0]
        return SynthesizedAnswer(
            answer=(
                f"Based on the selected data.gouv.fr sources, the strongest match for '{query}' is "
                f"'{primary.title}'. {primary.description}"
            ),
            limitations=[
                "This answer is grounded only in the selected sources returned by the retrieval step.",
                "Some freshness or coverage details may require inspecting the dataset metadata or resources directly.",
            ],
        )

    def _selection_reason(self, query: str, index: int) -> str:
        if index == 0:
            return f"This is the closest official match to the user query: '{query}'."
        return "This source provides useful complementary official context."
