import json

from app.config import settings
from app.schemas import AnalysisResult, MCPDatasetDetails, OrchestratorPlan, QueryRequest, QueryResponse, ScoutSelection, Source, SynthesizedAnswer
from app.services.analysis_service import DataAnalysisService
from app.services.gemini_service import GeminiService
from app.services.mcp_service import MCPService
from app.services.report_service import ReportService
from app.utils.logger import logger


class QueryOrchestrator:
    """Coordinates the Orchestrator, Dataset Scout, and Answer Synthesizer agents."""

    def __init__(self) -> None:
        self.gemini = GeminiService()
        self.mcp = MCPService()
        self.analysis_service = DataAnalysisService(self.mcp)
        self.report_service = ReportService()
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

        trace.append("Retrieving dataset details and resources...")
        selected_datasets = await self._load_selected_dataset_details(raw_results, selection.selected_sources)
        trace.append(f"Loaded details for {len(selected_datasets)} selected datasets.")

        trace.append("Reranking datasets after resource inspection...")
        ranked_datasets = self.analysis_service.rerank_datasets(request.query, selected_datasets)
        ranked_sources = self._rerank_selected_sources(selection.selected_sources, ranked_datasets)
        trace.append(f"Reranked {len(ranked_sources)} selected sources using resource quality signals.")

        trace.append("Analyzing selected dataset resources...")
        analysis = await self.analysis_service.analyze_query(request.query, ranked_datasets)
        trace.append(
            f"Analysis completed using {len(analysis.used_resources)} resource(s) with engine {analysis.analysis_engine}."
        )

        trace.append("Promoting analytically strongest datasets in final ranking...")
        expert_ranked_sources = self._promote_analyzed_sources(ranked_sources, analysis)
        trace.append(f"Promoted {len(expert_ranked_sources)} source(s) using inspected resource scores.")

        trace.append("Agent 3 (Answer Synthesizer) generating final response...")
        synth_prompt = self._load_prompt(
            "answer_synthesizer.txt",
            query=request.query,
            sources=json.dumps([source.model_dump(mode="json") for source in expert_ranked_sources], ensure_ascii=False, indent=2),
            analysis_context=analysis.model_dump_json(exclude={"resource_analyses"}, indent=2),
        )
        synthesis = self.gemini.generate_structured(
            synth_prompt,
            SynthesizedAnswer,
            self._build_mock_synthesis(request.query, expert_ranked_sources, analysis),
        )
        trace.append("Synthesis complete.")

        trace.append("Generating PDF and XLSX reports...")
        merged_limitations = self._merge_limitations(synthesis.limitations, analysis.limitations)
        report_artifacts = self.report_service.create_reports(
            user_query=request.query,
            selected_sources=expert_ranked_sources,
            analysis=analysis,
            answer=synthesis.answer,
            limitations=merged_limitations,
        )
        trace.append(f"Generated {len(report_artifacts)} report artifact(s).")

        return QueryResponse(
            user_query=request.query,
            selected_sources=expert_ranked_sources,
            answer=synthesis.answer,
            limitations=merged_limitations,
            trace=trace,
            analysis_engine=analysis.analysis_engine,
            analysis_summary=analysis.analysis_summary,
            key_findings=analysis.key_findings,
            data_coverage=analysis.data_coverage,
            dataset_row_count=analysis.dataset_row_count,
            dataset_columns=analysis.dataset_columns,
            descriptive_statistics=analysis.descriptive_statistics,
            regressions=analysis.regressions,
            charts=analysis.charts,
            used_resources=analysis.used_resources,
            report_artifacts=report_artifacts,
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

    def _build_mock_synthesis(self, query: str, sources: list[Source], analysis: AnalysisResult) -> SynthesizedAnswer:
        if not sources:
            return SynthesizedAnswer(
                answer="I could not identify a reliable official source for this query from the retrieved results.",
                limitations=[
                    "No retrieved source was relevant enough to support a grounded answer.",
                    "The workflow is intentionally restricted to retrieved official data.gouv.fr sources.",
                ],
            )

        primary = sources[0]
        analytical_sentence = analysis.key_findings[0] if analysis.key_findings else analysis.analysis_summary
        return SynthesizedAnswer(
            answer=(
                f"As a data analysis grounded in the selected data.gouv.fr resources, the strongest match for "
                f"'{query}' is '{primary.title}'. {analytical_sentence}"
            ),
            limitations=[
                "This answer is grounded only in the selected sources and analyzed resources returned by the retrieval step.",
                "Some findings are based on resource samples and may need broader dataset inspection for exhaustive conclusions.",
            ],
        )

    def _selection_reason(self, query: str, index: int) -> str:
        if index == 0:
            return f"This is the closest official match to the user query: '{query}'."
        return "This source provides useful complementary official context."

    async def _load_selected_dataset_details(
        self,
        raw_results: list[dict],
        selected_sources: list[Source],
    ) -> list[MCPDatasetDetails]:
        results_by_url = {item.get("url"): item for item in raw_results}
        datasets: list[MCPDatasetDetails] = []
        for source in selected_sources:
            raw_item = results_by_url.get(str(source.url))
            if raw_item is None:
                continue
            details_payload = await self.mcp.get_dataset_details(raw_item.get("id", ""))
            if details_payload is None:
                continue
            try:
                datasets.append(MCPDatasetDetails.model_validate(details_payload))
            except Exception as exc:
                logger.warning("Failed to validate dataset details for %s: %s", raw_item.get("id"), exc)
        return datasets

    def _merge_limitations(self, synthesis_limitations: list[str], analysis_limitations: list[str]) -> list[str]:
        merged: list[str] = []
        for limitation in synthesis_limitations + analysis_limitations:
            if limitation not in merged:
                merged.append(limitation)
        return merged

    def _rerank_selected_sources(
        self,
        selected_sources: list[Source],
        ranked_datasets: list[MCPDatasetDetails],
    ) -> list[Source]:
        source_by_url = {str(source.url): source for source in selected_sources}
        ordered: list[Source] = []
        for dataset in ranked_datasets:
            source = source_by_url.get(str(dataset.url))
            if source is not None:
                ordered.append(source)
        for source in selected_sources:
            if source not in ordered:
                ordered.append(source)
        return ordered

    def _promote_analyzed_sources(
        self,
        selected_sources: list[Source],
        analysis: AnalysisResult,
    ) -> list[Source]:
        if not analysis.resource_analyses:
            return selected_sources

        sources_by_title = {source.title: source for source in selected_sources}
        ordered: list[Source] = []
        for resource_analysis in analysis.resource_analyses:
            source = sources_by_title.get(resource_analysis.dataset_title)
            if source is not None and source not in ordered:
                ordered.append(
                    source.model_copy(
                        update={
                            "reason_for_selection": (
                                f"{source.reason_for_selection} Analytically prioritized because "
                                f"'{resource_analysis.resource_title}' exposed a high-quality {resource_analysis.format.upper()} resource."
                            ),
                            "confidence_score": min(1.0, max(source.confidence_score, 0.9)),
                        }
                    )
                )
        for source in selected_sources:
            if all(str(source.url) != str(existing.url) for existing in ordered):
                ordered.append(source)
        return ordered
