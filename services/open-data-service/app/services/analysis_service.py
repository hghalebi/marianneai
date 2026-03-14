import statistics
import unicodedata
from datetime import datetime
from typing import Any

from app.schemas import (
    AnalysisResult,
    AnalyticsComputation,
    AnalyticsChart,
    DescriptiveStatistic,
    MCPDatasetDetails,
    MCPResourceRecord,
    RegressionResult,
    ResourceAnalysis,
    ResourceSample,
    ResourceTable,
    UsedResource,
)
from app.services.code_interpreter_analytics import CodeInterpreterAnalyticsService
from app.services.mcp_service import MCPService
from app.utils.logger import logger


class DataAnalysisService:
    """Analyze retrieved dataset resources to produce grounded findings."""

    SUPPORTED_FORMATS = {"csv", "json", "geojson"}

    def __init__(self, mcp_service: MCPService) -> None:
        self.mcp = mcp_service
        self.analytics = CodeInterpreterAnalyticsService()

    async def analyze_query(self, query: str, datasets: list[MCPDatasetDetails]) -> AnalysisResult:
        limitations: list[str] = []
        resource_analyses: list[ResourceAnalysis] = []
        profiled_resources = 0

        for dataset in datasets:
            resource = self._pick_resource(query, dataset)
            if resource is None:
                limitations.append(f"No supported resource was found for dataset '{dataset.title}'.")
                continue

            table = await self.mcp.fetch_resource_table(resource)
            rows = table.rows
            if not rows:
                limitations.append(f"Resource '{resource.title}' could not be analyzed.")
                continue

            profiled_resources += 1
            sample = ResourceSample(
                rows=rows[: min(20, len(rows))],
                total_rows=table.total_rows,
                columns=table.columns,
                row_count_sampled=min(20, len(rows)),
            )
            computed = self.analytics.analyze(
                query=query,
                dataset_title=dataset.title,
                resource_title=resource.title,
                resource_format=resource.format,
                table=table,
            )
            resource_findings = self._dedupe_preserve_order(
                self._derive_findings(query, dataset, resource, sample) + computed.key_findings
            )
            coverage = computed.data_coverage or self._derive_coverage(sample)
            resource_score = self._inspected_resource_score(
                query=query,
                dataset=dataset,
                resource=resource,
                table=table,
                findings=resource_findings,
                coverage=coverage,
                computed=computed,
            )
            limitations.extend(computed.limitations)
            resource_analyses.append(
                ResourceAnalysis(
                    dataset_id=dataset.id,
                    dataset_title=dataset.title,
                    resource_id=resource.id,
                    resource_title=resource.title,
                    resource_url=resource.url,
                    format=resource.format,
                    score=resource_score,
                    coverage=coverage,
                    sample=sample,
                    table=table,
                    analysis_engine=computed.analysis_engine,
                    descriptive_statistics=computed.descriptive_statistics,
                    regressions=computed.regressions,
                    charts=computed.charts,
                    findings=resource_findings,
                )
            )

        if profiled_resources == 0:
            summary = "No resource could be profiled, so the answer is limited to dataset discovery metadata."
            if not limitations:
                limitations.append("No supported tabular resource was available for the selected datasets.")
            return AnalysisResult(
                analysis_summary=summary,
                key_findings=[],
                data_coverage="",
                used_resources=[],
                limitations=limitations,
                resource_analyses=[],
            )

        resource_analyses.sort(key=lambda analysis: analysis.score, reverse=True)
        findings: list[str] = []
        used_resources: list[UsedResource] = []
        coverage_parts: list[str] = []
        descriptive_statistics: list[DescriptiveStatistic] = []
        regressions: list[RegressionResult] = []
        charts: list[AnalyticsChart] = []
        analysis_engine = resource_analyses[0].analysis_engine if resource_analyses else "heuristic-local"
        dataset_row_count = resource_analyses[0].table.total_rows if resource_analyses else None
        dataset_columns = resource_analyses[0].table.columns if resource_analyses else []
        for analysis in resource_analyses:
            findings.extend(analysis.findings)
            used_resources.append(
                UsedResource(
                    dataset_title=analysis.dataset_title,
                    resource_title=analysis.resource_title,
                    resource_url=analysis.resource_url,
                    format=analysis.format,
                )
            )
            if analysis.coverage:
                coverage_parts.append(f"{analysis.dataset_title}: {analysis.coverage}")
            if not descriptive_statistics:
                descriptive_statistics = analysis.descriptive_statistics
            if not regressions:
                regressions = analysis.regressions
            if not charts:
                charts = analysis.charts

        unique_findings = self._dedupe_preserve_order(findings)[:8]
        summary = self._build_summary(query, profiled_resources, unique_findings, analysis_engine)
        return AnalysisResult(
            analysis_engine=analysis_engine,
            analysis_summary=summary,
            key_findings=unique_findings,
            data_coverage=" | ".join(coverage_parts[:3]),
            dataset_row_count=dataset_row_count,
            dataset_columns=dataset_columns,
            descriptive_statistics=descriptive_statistics,
            regressions=regressions,
            charts=charts,
            used_resources=used_resources,
            limitations=self._dedupe_preserve_order(limitations),
            resource_analyses=resource_analyses,
        )

    def rerank_datasets(self, query: str, datasets: list[MCPDatasetDetails]) -> list[MCPDatasetDetails]:
        return sorted(datasets, key=lambda dataset: self._dataset_score(query, dataset), reverse=True)

    def _pick_resource(self, query: str, dataset: MCPDatasetDetails) -> MCPResourceRecord | None:
        candidates = [resource for resource in dataset.resources if resource.format.lower() in self.SUPPORTED_FORMATS]
        if not candidates:
            return None
        ranked = sorted(
            candidates,
            key=lambda resource: self._resource_score(query, dataset, resource),
            reverse=True,
        )
        return ranked[0]

    def _derive_findings(
        self,
        query: str,
        dataset: MCPDatasetDetails,
        resource: MCPResourceRecord,
        sample: ResourceSample,
    ) -> list[str]:
        rows = sample.rows
        topic = self._infer_topic(query, dataset, resource, sample)
        lower_query = query.lower()
        if topic == "electric-mobility":
            return self._ev_findings(dataset.title, sample)
        if topic == "water-quality":
            return self._water_findings(dataset.title, sample)
        if topic == "public-transport":
            return self._transport_findings(dataset.title, sample)
        return self._generic_findings(dataset.title, resource.title, sample, lower_query)

    def _ev_findings(self, dataset_title: str, sample: ResourceSample) -> list[str]:
        rows = sample.rows
        columns = set(sample.columns or (rows[0].keys() if rows else []))
        normalized_columns = {self._normalize_label(column) for column in columns}
        if rows and any("registrations" in row for row in rows):
            registration_total = sum(self._as_number(row.get("registrations")) for row in rows)
            regions = sorted({str(row.get("region", "")).strip() for row in rows if row.get("region")})
            latest = self._latest_date(rows)
            findings = [
                f"{dataset_title} shows {int(registration_total)} electric vehicle registrations across {len(regions)} regions in the analyzed sample."
            ]
            if latest:
                findings.append(f"The latest registration period visible in the analyzed sample is {latest}.")
            return findings

        if {"id pdc local", "statut du point de recharge"} & normalized_columns:
            address_keys = [
                key for key in rows[0].keys() if self._normalize_label(key) in {"adresse", "adresse station", "adresse du point"}
            ] if rows else []
            point_ids = self._distinct_values(rows, [key for key in rows[0].keys() if self._normalize_label(key) in {"id pdc local", "id_pdc_local"}]) if rows else []
            statuses = self._distinct_values(rows, [key for key in rows[0].keys() if self._normalize_label(key) in {"statut du point de recharge", "statut"}]) if rows else []
            addresses = self._distinct_values(rows, address_keys)
            findings = [
                f"{dataset_title} contains {sample.row_count_sampled} sampled charging points from a live operational feed."
            ]
            if sample.total_rows:
                findings.append(f"The full live resource exposes {sample.total_rows} rows through the tabular API.")
            if point_ids:
                findings.append(f"The sample includes {len(point_ids)} distinct charging point identifiers.")
            if statuses:
                findings.append(f"Observed charging-point statuses in the sample include {', '.join(statuses[:3])}.")
            if addresses:
                findings.append(f"The sample spans locations such as {', '.join(addresses[:2])}.")
            latest = self._latest_date(rows)
            if latest:
                findings.append(f"The latest update visible in the analyzed sample is {latest}.")
            return findings

        if "nbre_pdc" in columns:
            charge_points = [self._as_number(row.get("nbre_pdc")) for row in rows if row.get("nbre_pdc") is not None]
            power_values = [self._as_number(row.get("puissance_nominale")) for row in rows if row.get("puissance_nominale") is not None]
            station_names = {str(row.get("nom_station", "")).strip() for row in rows if row.get("nom_station")}
            latest = self._latest_date(rows)
            findings = []
            if charge_points:
                findings.append(
                    f"{dataset_title} contains {sample.row_count_sampled} sampled records with {int(sum(charge_points))} charge points in total."
                )
                findings.append(
                    f"The sampled records average {statistics.mean(charge_points):.1f} charge points per station or point cluster."
                )
            if power_values:
                findings.append(
                    f"The average nominal power observed in the sample is {statistics.mean(power_values):.1f} kW."
                )
            if sample.total_rows:
                findings.append(f"The full resource reports {sample.total_rows} rows through the data.gouv tabular API.")
            if station_names:
                findings.append(f"The sample covers {len(station_names)} distinct station names.")
            if latest:
                findings.append(f"The latest update visible in the analyzed sample is {latest}.")
            return findings

        station_total = sum(self._as_number(row.get("stations")) for row in rows)
        charge_point_total = sum(self._as_number(row.get("charge_points")) for row in rows)
        regions = sorted({str(row.get("region", "")).strip() for row in rows if row.get("region")})
        latest = self._latest_date(rows)
        findings = [
            f"{dataset_title} reports {int(station_total)} charging stations across {len(regions)} regions in the analyzed sample.",
            f"The analyzed resource contains {int(charge_point_total)} charge points in total.",
        ]
        if latest:
            findings.append(f"The latest update visible in the analyzed sample is {latest}.")
        return findings

    def _water_findings(self, dataset_title: str, sample: ResourceSample) -> list[str]:
        rows = sample.rows
        compliance_values = [self._as_number(row.get("compliance_rate")) for row in rows if row.get("compliance_rate") is not None]
        measurements = sum(self._as_number(row.get("samples")) for row in rows)
        findings = [f"{dataset_title} includes {int(measurements)} analyzed samples in the profiled resource."]
        if compliance_values:
            findings.append(
                f"The average compliance rate in the analyzed sample is {statistics.mean(compliance_values):.1f}%."
            )
        latest = self._latest_date(rows)
        if latest:
            findings.append(f"The latest control date visible in the analyzed sample is {latest}.")
        return findings

    def _transport_findings(self, dataset_title: str, sample: ResourceSample) -> list[str]:
        rows = sample.rows
        punctuality_values = [self._as_number(row.get("punctuality_rate")) for row in rows if row.get("punctuality_rate") is not None]
        incidents = sum(self._as_number(row.get("incidents")) for row in rows)
        findings = [f"{dataset_title} reports {int(incidents)} incidents across the analyzed sample."]
        if punctuality_values:
            findings.append(
                f"The average punctuality rate in the analyzed sample is {statistics.mean(punctuality_values):.1f}%."
            )
        latest = self._latest_date(rows)
        if latest:
            findings.append(f"The latest reporting period visible in the sample is {latest}.")
        return findings

    def _generic_findings(
        self,
        dataset_title: str,
        resource_title: str,
        sample: ResourceSample,
        lower_query: str,
    ) -> list[str]:
        rows = sample.rows
        findings = [f"{dataset_title} / {resource_title} contains {sample.row_count_sampled} sampled records."]
        if sample.total_rows:
            findings.append(f"The full resource exposes {sample.total_rows} rows through the tabular API.")
        columns = sample.columns or (sorted(rows[0].keys()) if rows else [])
        if columns:
            findings.append(f"The analyzed resource exposes columns such as {', '.join(columns[:5])}.")
        if "latest" in lower_query or "recent" in lower_query:
            latest = self._latest_date(rows)
            if latest:
                findings.append(f"The latest date found in the sample is {latest}.")
        return findings

    def _derive_coverage(self, sample: ResourceSample) -> str:
        rows = sample.rows
        if not rows:
            return ""
        latest = self._latest_date(rows)
        earliest = self._earliest_date(rows)
        geographic_values = self._distinct_values(rows, ["region", "city", "department", "area", "consolidated_commune", "adresse_station"])
        if sample.total_rows:
            parts = [f"{sample.row_count_sampled} sampled rows out of {sample.total_rows} total rows"]
        else:
            parts = [f"{sample.row_count_sampled} rows analyzed"]
        if earliest or latest:
            if earliest and latest and earliest != latest:
                parts.append(f"time coverage {earliest} to {latest}")
            elif latest:
                parts.append(f"latest visible date {latest}")
        if geographic_values:
            parts.append(f"geographic sample {', '.join(geographic_values[:3])}")
        return ", ".join(parts)

    def _build_summary(self, query: str, profiled_resources: int, findings: list[str], analysis_engine: str) -> str:
        base = f"The backend analyzed {profiled_resources} resource(s) to answer: '{query}' using {analysis_engine}."
        if findings:
            return f"{base} Main analytical signal: {findings[0]}"
        return base

    def _infer_topic(
        self,
        query: str,
        dataset: MCPDatasetDetails,
        resource: MCPResourceRecord,
        sample: ResourceSample,
    ) -> str:
        explicit = str(dataset.metadata.get("topic", "")).lower()
        if explicit:
            return explicit

        searchable = " ".join(
            [
                query.lower(),
                dataset.title.lower(),
                dataset.description.lower(),
                resource.title.lower(),
                " ".join(sample.columns).lower(),
            ]
        )
        if any(token in searchable for token in ["irve", "borne", "recharge", "vehicule", "électrique", "electrique", "nbre_pdc", "puissance_nominale"]):
            return "electric-mobility"
        if any(token in searchable for token in ["eau", "water", "compliance", "potable"]):
            return "water-quality"
        if any(token in searchable for token in ["ponctual", "transport", "mobilite", "retard", "delay"]):
            return "public-transport"
        return ""

    def _resource_score(self, query: str, dataset: MCPDatasetDetails, resource: MCPResourceRecord) -> int:
        score = 0
        format_lower = resource.format.lower()
        title_lower = resource.title.lower()
        query_lower = query.lower()
        dataset_lower = f"{dataset.title} {dataset.description}".lower()

        if format_lower == "csv":
            score += 40
        elif format_lower == "json":
            score += 30
        elif format_lower == "geojson":
            score += 15

        resource_type = str(resource.metadata.get("resource_type", "")).lower()
        if resource_type == "main":
            score += 25
        if resource_type == "documentation":
            score -= 40

        if any(token in title_lower for token in ["temps réel", "temps reel", "disponibilite", "availability"]):
            score -= 10
        if any(token in title_lower for token in ["documentation", "schema", "ref-table", "openapi"]):
            score -= 50
        if any(token in title_lower for token in ["consolidation", "summary", "national", "data", "irve"]):
            score += 10
        if "lidl" in dataset_lower and "csv" in format_lower:
            score += 10
        if any(token in query_lower for token in ["latest", "recent", "update"]) and resource.last_modified:
            score += 5
        if resource.metadata.get("size"):
            score += 2
        return score

    def _dataset_score(self, query: str, dataset: MCPDatasetDetails) -> int:
        dataset_lower = f"{dataset.title} {dataset.description}".lower()
        query_lower = query.lower()
        score = max((self._resource_score(query, dataset, resource) for resource in dataset.resources), default=0)

        if any(token in dataset_lower for token in ["base nationale", "national", "france", "consolide", "consolidation"]):
            score += 25
        if any(token in dataset_lower for token in ["temps reel", "temps réel", "disponibilite", "live"]):
            score -= 10
        if "france" in query_lower and any(token in dataset_lower for token in ["paris", "lyon"]):
            score -= 8
        if any(token in dataset.organization.lower() for token in ["data.gouv", "minist", "etat", "gouvernement"]):
            score += 10
        if dataset.metadata.get("last_update"):
            score += 5
        return score

    def _inspected_resource_score(
        self,
        query: str,
        dataset: MCPDatasetDetails,
        resource: MCPResourceRecord,
        table: ResourceTable,
        findings: list[str],
        coverage: str,
        computed: AnalyticsComputation,
    ) -> int:
        score = self._resource_score(query, dataset, resource)
        columns = table.columns or (list(table.rows[0].keys()) if table.rows else [])
        normalized_columns = {self._normalize_label(column) for column in columns}
        sample = ResourceSample(
            rows=table.rows[: min(20, len(table.rows))],
            total_rows=table.total_rows,
            columns=table.columns,
            row_count_sampled=min(20, len(table.rows)),
        )
        topic = self._infer_topic(query, dataset, resource, sample)

        if table.rows:
            score += 20
        if table.full_download_used:
            score += 20
        if table.total_rows:
            score += min(20, table.total_rows // 250)
        if len(columns) >= 5:
            score += 8
        if len(columns) >= 10:
            score += 5
        if findings:
            score += min(12, len(findings) * 3)
        if coverage:
            score += 6
        if computed.descriptive_statistics:
            score += 10
        if computed.regressions:
            score += 12
        if computed.charts:
            score += 8

        if topic == "electric-mobility":
            if {"nbre pdc", "id pdc local", "statut du point de recharge"} & normalized_columns:
                score += 18
            if {"nom station", "adresse station", "puissance nominale"} & normalized_columns:
                score += 10
        if any(token in query.lower() for token in ["france", "national"]) and any(
            token in f"{dataset.title} {dataset.description}".lower() for token in ["france", "national", "consolide", "consolidation"]
        ):
            score += 14
        if any(token in query.lower() for token in ["latest", "recent", "update"]) and self._latest_date(table.rows):
            score += 10
        return score

    def _normalize_label(self, value: str) -> str:
        ascii_value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode("ascii")
        cleaned = "".join(character if character.isalnum() or character == " " else " " for character in ascii_value.lower())
        return " ".join(cleaned.split())

    def _latest_date(self, rows: list[dict[str, Any]]) -> str | None:
        dates = self._extract_dates(rows)
        return dates[-1].date().isoformat() if dates else None

    def _earliest_date(self, rows: list[dict[str, Any]]) -> str | None:
        dates = self._extract_dates(rows)
        return dates[0].date().isoformat() if dates else None

    def _extract_dates(self, rows: list[dict[str, Any]]) -> list[datetime]:
        parsed_dates: list[datetime] = []
        for row in rows:
            for value in row.values():
                parsed = self._parse_date(value)
                if parsed is not None:
                    parsed_dates.append(parsed)
        return sorted(parsed_dates)

    def _parse_date(self, value: Any) -> datetime | None:
        if not isinstance(value, str):
            return None
        text = value.strip()
        if not text:
            return None
        normalized = text.replace("Z", "+00:00")
        for candidate in (normalized, normalized[:10]):
            try:
                if len(candidate) == 10:
                    return datetime.strptime(candidate, "%Y-%m-%d")
                return datetime.fromisoformat(candidate)
            except ValueError:
                continue
        return None

    def _distinct_values(self, rows: list[dict[str, Any]], keys: list[str]) -> list[str]:
        values: list[str] = []
        for row in rows:
            for key in keys:
                value = row.get(key)
                if value:
                    text = str(value).strip()
                    if text and text not in values:
                        values.append(text)
        return values

    def _as_number(self, value: Any) -> float:
        if value is None:
            return 0.0
        if isinstance(value, (int, float)):
            return float(value)
        try:
            return float(str(value).replace(",", "."))
        except ValueError:
            logger.debug("Failed to parse numeric value: %s", value)
            return 0.0

    def _dedupe_preserve_order(self, values: list[str]) -> list[str]:
        seen: set[str] = set()
        deduped: list[str] = []
        for value in values:
            if value not in seen:
                seen.add(value)
                deduped.append(value)
        return deduped
