import json
import math
import statistics
from collections import Counter, defaultdict
from datetime import datetime
from typing import Any

try:
    from google import genai
    from google.genai import types
except ImportError:  # pragma: no cover
    genai = None
    types = None

from app.config import settings
from app.schemas import AnalyticsChart, AnalyticsComputation, DescriptiveStatistic, RegressionResult, ResourceTable
from app.utils.logger import logger


class CodeInterpreterAnalyticsService:
    """Compute structured analytics locally or with Vertex AI code execution."""

    def __init__(self) -> None:
        self.vertex_enabled = bool(
            settings.enable_vertex_code_execution
            and settings.google_cloud_project
            and genai is not None
            and types is not None
        )
        self.client = None
        if self.vertex_enabled:
            try:
                self.client = genai.Client(
                    vertexai=True,
                    project=settings.google_cloud_project,
                    location=settings.google_cloud_location,
                )
            except Exception as exc:  # pragma: no cover
                logger.warning("Vertex AI client initialization failed: %s. Falling back to local analytics.", exc)
                self.vertex_enabled = False

    def analyze(
        self,
        *,
        query: str,
        dataset_title: str,
        resource_title: str,
        resource_format: str,
        table: ResourceTable,
    ) -> AnalyticsComputation:
        if self.vertex_enabled and table.text and len(table.text) <= settings.max_vertex_buffer_chars:
            computation = self._analyze_with_vertex(
                query=query,
                dataset_title=dataset_title,
                resource_title=resource_title,
                resource_format=resource_format,
                table=table,
            )
            if computation is not None:
                return computation
        return self._analyze_locally(
            query=query,
            dataset_title=dataset_title,
            resource_title=resource_title,
            resource_format=resource_format,
            table=table,
        )

    def _analyze_with_vertex(
        self,
        *,
        query: str,
        dataset_title: str,
        resource_title: str,
        resource_format: str,
        table: ResourceTable,
    ) -> AnalyticsComputation | None:
        if self.client is None or types is None:
            return None

        prompt = (
            "You are a deterministic data analysis engine. "
            "Use code execution to analyze the provided dataset buffer. "
            "Return only valid JSON. "
            "Compute descriptive statistics for numeric columns, up to three linear regressions "
            "between meaningful numeric pairs, and chart payloads directly renderable in Recharts. "
            "Each chart must include chart_id, title, chart_type, description, x_key, y_keys, data. "
            "Prefer line charts for time series, bar charts for categorical aggregations, and scatter charts for numeric pairs. "
            "Be explicit about limitations.\n\n"
            f"User query: {query}\n"
            f"Dataset title: {dataset_title}\n"
            f"Resource title: {resource_title}\n"
            f"Resource format: {resource_format}\n"
            f"Full download used: {table.full_download_used}\n"
            f"Row count analyzed: {table.row_count_analyzed}\n"
            "Expected JSON schema keys: "
            "analysis_engine, analysis_summary, key_findings, data_coverage, row_count, columns, "
            "descriptive_statistics, regressions, charts, limitations.\n\n"
            "Dataset buffer:\n"
            f"{table.text}"
        )

        try:
            response = self.client.models.generate_content(
                model=settings.vertex_code_execution_model,
                contents=prompt,
                config=types.GenerateContentConfig(
                    temperature=0,
                    response_mime_type="application/json",
                    tools=[types.Tool(code_execution=types.ToolCodeExecution())],
                ),
            )
            payload = getattr(response, "text", "") or ""
            if not payload:
                return None
            data = json.loads(payload)
            data["analysis_engine"] = "vertex-code-execution"
            return AnalyticsComputation.model_validate(data)
        except Exception as exc:  # pragma: no cover
            logger.warning("Vertex code execution analytics failed: %s. Falling back to local analytics.", exc)
            return None

    def _analyze_locally(
        self,
        *,
        query: str,
        dataset_title: str,
        resource_title: str,
        resource_format: str,
        table: ResourceTable,
    ) -> AnalyticsComputation:
        rows = table.rows
        if not rows:
            return AnalyticsComputation(
                analysis_engine="heuristic-local",
                analysis_summary=f"No analyzable rows were available for {dataset_title}.",
                limitations=["The resource did not expose any structured rows after parsing."],
            )

        columns = table.columns or list(rows[0].keys())
        numeric_columns = self._numeric_columns(rows, columns)
        date_columns = self._date_columns(rows, columns)
        categorical_columns = self._categorical_columns(rows, columns, numeric_columns, date_columns)

        descriptive_statistics = self._descriptive_statistics(rows, numeric_columns)
        regressions = self._regressions(rows, numeric_columns)
        charts = self._charts(rows, numeric_columns, date_columns, categorical_columns)
        key_findings = self._key_findings(
            query=query,
            dataset_title=dataset_title,
            resource_title=resource_title,
            table=table,
            descriptive_statistics=descriptive_statistics,
            regressions=regressions,
            charts=charts,
        )
        coverage = self._coverage(rows, table, date_columns, categorical_columns)
        limitations = []
        if not table.full_download_used:
            limitations.append("The analytics engine worked on a structured sample because a full download was not considered reasonable.")
        if not regressions:
            limitations.append("No statistically meaningful linear regression was available from the detected numeric columns.")
        if not charts:
            limitations.append("No generic chart payload could be generated from the detected columns.")

        summary = (
            f"Local analytics processed {table.row_count_analyzed} row(s) from '{resource_title}' "
            f"for dataset '{dataset_title}' using {resource_format.upper()} parsing."
        )
        return AnalyticsComputation(
            analysis_engine="heuristic-local",
            analysis_summary=summary,
            key_findings=key_findings,
            data_coverage=coverage,
            row_count=table.total_rows or table.row_count_analyzed,
            columns=columns,
            descriptive_statistics=descriptive_statistics,
            regressions=regressions,
            charts=charts,
            limitations=limitations,
        )

    def _numeric_columns(self, rows: list[dict[str, Any]], columns: list[str]) -> list[str]:
        scored_columns: list[tuple[int, str]] = []
        for column in columns:
            values = [self._to_float(row.get(column)) for row in rows if row.get(column) not in (None, "")]
            valid = [value for value in values if value is not None]
            if len(valid) >= 3 and len(valid) >= max(3, int(len(values) * 0.6)):
                score = self._numeric_column_score(column, valid, len(rows))
                if score > 0:
                    scored_columns.append((score, column))
        scored_columns.sort(reverse=True)
        return [column for _, column in scored_columns[:8]]

    def _date_columns(self, rows: list[dict[str, Any]], columns: list[str]) -> list[str]:
        date_columns: list[str] = []
        for column in columns:
            parsed = [self._parse_date(row.get(column)) for row in rows if row.get(column)]
            valid = [value for value in parsed if value is not None]
            if len(valid) >= 3 and len(valid) >= max(3, int(len(parsed) * 0.6)):
                date_columns.append(column)
        return date_columns[:4]

    def _categorical_columns(
        self,
        rows: list[dict[str, Any]],
        columns: list[str],
        numeric_columns: list[str],
        date_columns: list[str],
    ) -> list[str]:
        categorical_columns: list[str] = []
        ignored = set(numeric_columns) | set(date_columns)
        for column in columns:
            if column in ignored:
                continue
            if any(token in column.lower() for token in ["id_", "id ", "url", "email", "telephone", "contact"]):
                continue
            values = [str(row.get(column)).strip() for row in rows if row.get(column) not in (None, "")]
            distinct = {value for value in values if value}
            if 1 < len(distinct) <= min(20, max(3, len(rows))):
                categorical_columns.append(column)
        return categorical_columns[:6]

    def _descriptive_statistics(
        self,
        rows: list[dict[str, Any]],
        numeric_columns: list[str],
    ) -> list[DescriptiveStatistic]:
        statistics_rows: list[DescriptiveStatistic] = []
        for column in numeric_columns[:6]:
            values = [self._to_float(row.get(column)) for row in rows]
            valid = [value for value in values if value is not None]
            if not valid:
                continue
            stddev = statistics.pstdev(valid) if len(valid) > 1 else 0.0
            statistics_rows.append(
                DescriptiveStatistic(
                    column=column,
                    non_null_count=len(valid),
                    mean=round(statistics.fmean(valid), 4),
                    min=round(min(valid), 4),
                    max=round(max(valid), 4),
                    median=round(statistics.median(valid), 4),
                    stddev=round(stddev, 4),
                )
            )
        return statistics_rows

    def _regressions(
        self,
        rows: list[dict[str, Any]],
        numeric_columns: list[str],
    ) -> list[RegressionResult]:
        regressions: list[RegressionResult] = []
        for feature_x in numeric_columns[:4]:
            for feature_y in numeric_columns[:4]:
                if feature_x == feature_y:
                    continue
                pairs = []
                for row in rows:
                    x_value = self._to_float(row.get(feature_x))
                    y_value = self._to_float(row.get(feature_y))
                    if x_value is not None and y_value is not None:
                        pairs.append((x_value, y_value))
                if len(pairs) < 3:
                    continue
                xs = [pair[0] for pair in pairs]
                ys = [pair[1] for pair in pairs]
                x_mean = statistics.fmean(xs)
                y_mean = statistics.fmean(ys)
                denominator = sum((x_value - x_mean) ** 2 for x_value in xs)
                if denominator == 0:
                    continue
                slope = sum((x_value - x_mean) * (y_value - y_mean) for x_value, y_value in pairs) / denominator
                intercept = y_mean - slope * x_mean
                ss_total = sum((y_value - y_mean) ** 2 for y_value in ys)
                ss_residual = sum((y_value - (slope * x_value + intercept)) ** 2 for x_value, y_value in pairs)
                r_squared = 1 - (ss_residual / ss_total) if ss_total else 0.0
                regressions.append(
                    RegressionResult(
                        feature_x=feature_x,
                        feature_y=feature_y,
                        slope=round(slope, 6),
                        intercept=round(intercept, 6),
                        r_squared=round(r_squared, 6),
                        sample_size=len(pairs),
                    )
                )
        regressions = [item for item in regressions if abs(item.r_squared) >= 0.05]
        regressions.sort(key=lambda item: abs(item.r_squared), reverse=True)
        unique_pairs: list[RegressionResult] = []
        seen: set[tuple[str, str]] = set()
        for item in regressions:
            pair = (item.feature_x, item.feature_y)
            if pair in seen:
                continue
            seen.add(pair)
            unique_pairs.append(item)
            if len(unique_pairs) == 3:
                break
        return unique_pairs

    def _charts(
        self,
        rows: list[dict[str, Any]],
        numeric_columns: list[str],
        date_columns: list[str],
        categorical_columns: list[str],
    ) -> list[AnalyticsChart]:
        charts: list[AnalyticsChart] = []

        if date_columns and numeric_columns:
            date_column = date_columns[0]
            numeric_column = numeric_columns[0]
            grouped: dict[str, list[float]] = defaultdict(list)
            for row in rows:
                date_value = self._parse_date(row.get(date_column))
                numeric_value = self._to_float(row.get(numeric_column))
                if date_value is None or numeric_value is None:
                    continue
                grouped[date_value.date().isoformat()].append(numeric_value)
            if grouped:
                data = [
                    {"date": key, numeric_column: round(statistics.fmean(values), 4)}
                    for key, values in sorted(grouped.items())
                ]
                charts.append(
                    AnalyticsChart(
                        chart_id=f"time-series-{self._slugify(date_column)}-{self._slugify(numeric_column)}",
                        title=f"{numeric_column} over time",
                        chart_type="line",
                        description=f"Average {numeric_column} grouped by {date_column}.",
                        x_key="date",
                        y_keys=[numeric_column],
                        data=data[:60],
                    )
                )

        if categorical_columns:
            category_column = categorical_columns[0]
            if numeric_columns:
                numeric_column = numeric_columns[0]
                grouped_numeric: dict[str, float] = defaultdict(float)
                for row in rows:
                    category = str(row.get(category_column, "")).strip()
                    numeric_value = self._to_float(row.get(numeric_column))
                    if category and numeric_value is not None:
                        grouped_numeric[category] += numeric_value
                if grouped_numeric:
                    top_values = sorted(grouped_numeric.items(), key=lambda item: item[1], reverse=True)[:10]
                    charts.append(
                        AnalyticsChart(
                            chart_id=f"bar-{self._slugify(category_column)}-{self._slugify(numeric_column)}",
                            title=f"{numeric_column} by {category_column}",
                            chart_type="bar",
                            description=f"Aggregated {numeric_column} by {category_column}.",
                            x_key=category_column,
                            y_keys=[numeric_column],
                            data=[{category_column: key, numeric_column: round(value, 4)} for key, value in top_values],
                        )
                    )
            else:
                counts = Counter(str(row.get(category_column, "")).strip() for row in rows if row.get(category_column))
                if counts:
                    charts.append(
                        AnalyticsChart(
                            chart_id=f"count-{self._slugify(category_column)}",
                            title=f"Count by {category_column}",
                            chart_type="bar",
                            description=f"Record counts by {category_column}.",
                            x_key=category_column,
                            y_keys=["count"],
                            data=[{category_column: key, "count": value} for key, value in counts.most_common(10)],
                        )
                    )

        if len(numeric_columns) >= 2:
            x_column = numeric_columns[0]
            y_column = numeric_columns[1]
            scatter_rows = []
            for row in rows:
                x_value = self._to_float(row.get(x_column))
                y_value = self._to_float(row.get(y_column))
                if x_value is not None and y_value is not None:
                    scatter_rows.append({x_column: round(x_value, 4), y_column: round(y_value, 4)})
            if scatter_rows:
                charts.append(
                    AnalyticsChart(
                        chart_id=f"scatter-{self._slugify(x_column)}-{self._slugify(y_column)}",
                        title=f"{y_column} vs {x_column}",
                        chart_type="scatter",
                        description=f"Scatter plot of {y_column} against {x_column}.",
                        x_key=x_column,
                        y_keys=[y_column],
                        data=scatter_rows[:200],
                    )
                )

        return charts[:3]

    def _numeric_column_score(self, column: str, values: list[float], row_count: int) -> int:
        name = column.lower()
        if any(
            token in name
            for token in [
                "id",
                "code",
                "siren",
                "siret",
                "telephone",
                "phone",
                "pdl",
                "postal",
                "zip",
                "lat",
                "lon",
                "coord",
            ]
        ):
            return -100
        score = 10
        if any(
            token in name
            for token in [
                "count",
                "nb",
                "nombre",
                "nbre",
                "power",
                "puissance",
                "charge",
                "station",
                "registration",
                "samples",
                "incident",
                "rate",
                "score",
                "price",
                "amount",
            ]
        ):
            score += 25
        distinct_ratio = len({round(value, 6) for value in values}) / max(1, len(values))
        if distinct_ratio > 0.95 and row_count > 50:
            score -= 20
        if len({round(value, 6) for value in values}) == 1:
            score -= 50
        return score

    def _key_findings(
        self,
        *,
        query: str,
        dataset_title: str,
        resource_title: str,
        table: ResourceTable,
        descriptive_statistics: list[DescriptiveStatistic],
        regressions: list[RegressionResult],
        charts: list[AnalyticsChart],
    ) -> list[str]:
        findings = [
            f"{dataset_title} / {resource_title} analyzed {table.row_count_analyzed} row(s)."
        ]
        if table.total_rows and table.total_rows != table.row_count_analyzed:
            findings.append(f"The source reports {table.total_rows} row(s) in total.")
        if descriptive_statistics:
            top_stat = descriptive_statistics[0]
            if top_stat.mean is not None:
                findings.append(
                    f"The numeric column '{top_stat.column}' has mean {top_stat.mean}, median {top_stat.median}, and range {top_stat.min} to {top_stat.max}."
                )
        if regressions:
            strongest = regressions[0]
            findings.append(
                f"The strongest detected linear relationship is {strongest.feature_y} vs {strongest.feature_x} with R^2={strongest.r_squared}."
            )
        if charts:
            findings.append(f"{len(charts)} frontend-ready chart payload(s) were generated for direct rendering.")
        if "latest" in query.lower() or "recent" in query.lower():
            findings.append("The analysis included date detection to support freshness-oriented questions when date fields were available.")
        return findings[:6]

    def _coverage(
        self,
        rows: list[dict[str, Any]],
        table: ResourceTable,
        date_columns: list[str],
        categorical_columns: list[str],
    ) -> str:
        parts = [f"{table.row_count_analyzed} row(s) analyzed"]
        if table.total_rows and table.total_rows != table.row_count_analyzed:
            parts.append(f"{table.total_rows} total row(s) available")
        if date_columns:
            dates = [self._parse_date(row.get(date_columns[0])) for row in rows]
            valid_dates = sorted([value for value in dates if value is not None])
            if valid_dates:
                parts.append(f"time coverage {valid_dates[0].date().isoformat()} to {valid_dates[-1].date().isoformat()}")
        if categorical_columns:
            categories = {str(row.get(categorical_columns[0], "")).strip() for row in rows if row.get(categorical_columns[0])}
            if categories:
                parts.append(f"{len(categories)} distinct values for {categorical_columns[0]}")
        return ", ".join(parts)

    def _to_float(self, value: Any) -> float | None:
        if value in (None, ""):
            return None
        if isinstance(value, bool):
            return float(value)
        if isinstance(value, (int, float)):
            numeric = float(value)
            if math.isfinite(numeric):
                return numeric
            return None
        try:
            numeric = float(str(value).replace(",", "."))
            if math.isfinite(numeric):
                return numeric
        except ValueError:
            return None
        return None

    def _parse_date(self, value: Any) -> datetime | None:
        if not isinstance(value, str):
            return None
        cleaned = value.strip().replace("Z", "+00:00")
        if not cleaned:
            return None
        candidates = [cleaned, cleaned[:10]]
        for candidate in candidates:
            try:
                if len(candidate) == 10:
                    return datetime.strptime(candidate, "%Y-%m-%d")
                return datetime.fromisoformat(candidate)
            except ValueError:
                continue
        return None

    def _slugify(self, value: str) -> str:
        cleaned = "".join(character.lower() if character.isalnum() else "-" for character in value)
        while "--" in cleaned:
            cleaned = cleaned.replace("--", "-")
        return cleaned.strip("-") or "value"
