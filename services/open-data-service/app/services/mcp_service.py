import asyncio
import csv
import io
import json
import re
from typing import Any, Dict, List

import httpx

try:
    from mcp import ClientSession
    from mcp.client.streamable_http import streamable_http_client
    from mcp.types import TextContent
except ImportError:  # pragma: no cover
    ClientSession = None
    streamable_http_client = None
    TextContent = None

from app.config import settings
from app.schemas import MCPDatasetDetails, MCPDatasetRecord, MCPResourceRecord, ResourceSample, ResourceTable
from app.utils.logger import logger


class MCPService:
    """Adapter for the official data.gouv MCP server, with a mock fallback."""

    def __init__(self) -> None:
        self.use_mock = settings.use_mock_mcp
        self.mcp_url = settings.mcp_server_url
        if not self.use_mock and (ClientSession is None or streamable_http_client is None):
            logger.warning("MCP SDK is not available. Falling back to mock mode.")
            self.use_mock = True

    async def search_data_gouv(self, queries: List[str]) -> List[Dict[str, Any]]:
        if self.use_mock:
            logger.info("Mocking MCP search for queries: %s", queries)
            return self._search_mock_data(queries)

        query_text = " ".join(query.strip() for query in queries if query.strip())
        try:
            payload = await self._call_tool(
                "search_datasets",
                {
                    "query": query_text,
                    "page": 1,
                    "page_size": 5,
                },
            )
            normalized_results = self._normalize_search_results(payload)
            if normalized_results:
                return normalized_results
            logger.warning("Live MCP returned no normalized dataset results. Falling back to mock mode.")
        except Exception as exc:
            logger.error("Live MCP search failed: %s. Falling back to mock mode.", exc)
        return self._search_mock_data(queries)

    async def get_dataset_details(self, dataset_id: str) -> Dict[str, Any] | None:
        if self.use_mock:
            return self._mock_dataset_details().get(dataset_id)

        try:
            dataset_payload = await self._call_tool("get_dataset_info", {"dataset_id": dataset_id})
            resources_payload = await self._call_tool("list_dataset_resources", {"dataset_id": dataset_id})
            normalized_dataset = self._normalize_dataset_details(dataset_payload)
            normalized_resources = self._normalize_resources(resources_payload)
            if normalized_dataset is None:
                return None
            normalized_dataset["resources"] = normalized_resources
            normalized_dataset.setdefault("metadata", {})
            normalized_dataset["metadata"]["source"] = "mcp-live"
            return MCPDatasetDetails.model_validate(normalized_dataset).model_dump(mode="json")
        except Exception as exc:
            logger.error("Dataset details lookup failed for %s: %s", dataset_id, exc)
            return None

    async def fetch_resource_sample(self, resource: MCPResourceRecord) -> ResourceSample:
        if self.use_mock:
            rows = self._normalize_rows(self._mock_resource_rows().get(resource.id, []))
            return ResourceSample(
                rows=rows,
                total_rows=len(rows),
                columns=self._extract_columns(rows),
                row_count_sampled=len(rows),
            )

        try:
            query_payload = await self._call_tool(
                "query_resource_data",
                {
                    "resource_id": resource.id,
                    "question": "Return a small representative sample of rows for backend analysis, with values preserved.",
                    "page": 1,
                    "page_size": 5,
                },
            )
            sample = self._parse_query_resource_data_text(query_payload.get("result", ""))
            if sample.rows:
                return sample
        except Exception as exc:
            logger.warning("query_resource_data failed for %s: %s", resource.id, exc)

        resolved_url = str(resource.url)
        try:
            resource_payload = await self._call_tool("get_resource_info", {"resource_id": resource.id})
            normalized_resource = self._normalize_resource_info(resource_payload)
            if normalized_resource is not None:
                resolved_url = normalized_resource.get("url", resolved_url)
        except Exception as exc:
            logger.warning("Resource info lookup failed for %s: %s", resource.id, exc)

        try:
            async with httpx.AsyncClient(timeout=settings.http_timeout_seconds, follow_redirects=True) as client:
                response = await client.get(resolved_url)
                response.raise_for_status()
        except httpx.HTTPError as exc:
            logger.error("Resource fetch failed for %s: %s", resolved_url, exc)
            return ResourceSample()

        rows = self._normalize_rows(self._parse_resource_content(resource.format, response.text))
        return ResourceSample(
            rows=rows,
            total_rows=len(rows) if rows else None,
            columns=self._extract_columns(rows),
            row_count_sampled=len(rows),
        )

    async def fetch_resource_table(self, resource: MCPResourceRecord) -> ResourceTable:
        if self.use_mock:
            rows = self._mock_resource_rows().get(resource.id, [])
            serialized_rows = self._normalize_rows(rows)
            return ResourceTable(
                text=self._serialize_rows(resource.format, serialized_rows),
                rows=serialized_rows,
                total_rows=len(serialized_rows),
                columns=self._extract_columns(serialized_rows),
                row_count_analyzed=len(serialized_rows),
                content_bytes=len(self._serialize_rows(resource.format, serialized_rows).encode("utf-8")),
                full_download_used=True,
            )

        sample = await self.fetch_resource_sample(resource)
        sample_rows = self._normalize_rows(sample.rows)
        sample_table = ResourceTable(
            text=self._serialize_rows(resource.format, sample_rows),
            rows=sample_rows,
            total_rows=sample.total_rows,
            columns=self._extract_columns(sample_rows) or sample.columns,
            row_count_analyzed=sample.row_count_sampled,
            content_bytes=len(self._serialize_rows(resource.format, sample_rows).encode("utf-8")),
            full_download_used=False,
        )

        if not settings.enable_full_resource_download:
            return sample_table

        declared_size = self._parse_size_to_bytes(str(resource.metadata.get("size", "")))
        if declared_size is not None and declared_size > settings.max_full_resource_bytes:
            logger.info(
                "Skipping full download for resource %s because declared size %s exceeds limit %s.",
                resource.id,
                declared_size,
                settings.max_full_resource_bytes,
            )
            return sample_table

        resolved_url = await self._resolve_resource_url(resource)
        if not resolved_url:
            return sample_table

        payload = await self._download_resource_text(resolved_url)
        if payload is None:
            return sample_table
        text, content_bytes = payload
        if content_bytes > settings.max_full_resource_bytes:
            logger.info(
                "Skipping full analysis for resource %s because downloaded content %s exceeds limit %s.",
                resource.id,
                content_bytes,
                settings.max_full_resource_bytes,
            )
            return sample_table

        try:
            rows = self._normalize_rows(self._parse_resource_content(resource.format, text))
        except Exception as exc:
            logger.warning("Failed to parse full resource %s: %s", resource.id, exc)
            return sample_table

        if not rows:
            return sample_table

        actual_total_rows = len(rows)
        if len(rows) > settings.max_full_resource_rows:
            logger.info(
                "Capping full analysis rows for resource %s from %s to %s.",
                resource.id,
                len(rows),
                settings.max_full_resource_rows,
            )
            rows = rows[: settings.max_full_resource_rows]

        columns = self._extract_columns(rows)
        return ResourceTable(
            text=text,
            rows=rows,
            total_rows=actual_total_rows,
            columns=columns,
            row_count_analyzed=len(rows),
            content_bytes=content_bytes,
            full_download_used=True,
        )

    async def _call_tool(self, tool_name: str, arguments: Dict[str, Any]) -> Dict[str, Any]:
        last_error: Exception | None = None
        for attempt in range(2):
            try:
                async with streamable_http_client(self.mcp_url) as streams:
                    read, write, _ = streams
                    async with ClientSession(read, write) as session:
                        await session.initialize()
                        result = await session.call_tool(tool_name, arguments=arguments)
                        payload = self._extract_tool_payload(result)
                        if isinstance(payload, dict):
                            return payload
                        return {"data": payload}
            except Exception as exc:
                last_error = exc
                if attempt == 0:
                    await asyncio.sleep(0.5)
                    continue
                raise
        raise RuntimeError(f"MCP call failed for {tool_name}: {last_error}")

    def _extract_tool_payload(self, result: Any) -> Any:
        structured = getattr(result, "structuredContent", None)
        if structured is not None:
            return structured

        texts: list[str] = []
        for item in getattr(result, "content", []):
            text_value = getattr(item, "text", None)
            if text_value:
                texts.append(text_value)

        if not texts:
            return {}

        joined = "\n".join(texts).strip()
        try:
            return json.loads(joined)
        except json.JSONDecodeError:
            return {"text": joined}

    def _normalize_search_results(self, payload: Dict[str, Any]) -> List[Dict[str, Any]]:
        raw_items = self._extract_items(payload)
        if not raw_items and isinstance(payload.get("result"), str):
            raw_items = self._parse_search_result_text(payload["result"])
        normalized_results: List[Dict[str, Any]] = []
        for item in raw_items:
            if not isinstance(item, dict):
                continue
            normalized = {
                "id": item.get("id") or item.get("dataset_id") or "",
                "title": item.get("title") or item.get("name") or "Untitled dataset",
                "description": item.get("description") or item.get("summary") or "",
                "url": item.get("url") or item.get("page") or item.get("uri"),
                "organization": self._organization_name(item),
                "metadata": {
                    "tags": item.get("tags", []),
                    "last_update": item.get("last_update") or item.get("updated_at") or "",
                    "source": "mcp-live",
                },
            }
            try:
                normalized_results.append(MCPDatasetRecord.model_validate(normalized).model_dump(mode="json"))
            except Exception as exc:
                logger.warning("Skipping invalid live MCP dataset item: %s", exc)
        return normalized_results

    def _normalize_dataset_details(self, payload: Dict[str, Any]) -> Dict[str, Any] | None:
        item = self._unwrap_single_item(payload)
        if item is None and isinstance(payload.get("result"), str):
            item = self._parse_dataset_info_text(payload["result"])
        if not isinstance(item, dict):
            return None
        normalized = {
            "id": item.get("id") or item.get("dataset_id") or "",
            "title": item.get("title") or item.get("name") or "Untitled dataset",
            "description": item.get("description") or item.get("summary") or "",
            "url": item.get("url") or item.get("page") or item.get("uri"),
            "organization": self._organization_name(item),
            "metadata": {
                "tags": item.get("tags", []),
                "license": item.get("license") or "",
                "last_update": item.get("last_update") or item.get("updated_at") or "",
            },
        }
        return normalized

    def _normalize_resources(self, payload: Dict[str, Any]) -> List[Dict[str, Any]]:
        raw_items = self._extract_items(payload)
        if not raw_items and isinstance(payload.get("result"), str):
            raw_items = self._parse_resources_text(payload["result"])
        resources: List[Dict[str, Any]] = []
        for item in raw_items:
            if not isinstance(item, dict):
                continue
            normalized = {
                "id": item.get("id") or item.get("resource_id") or "",
                "title": item.get("title") or item.get("name") or "Untitled resource",
                "url": item.get("url") or item.get("download_url") or item.get("uri"),
                "format": (item.get("format") or item.get("filetype") or "").lower(),
                "description": item.get("description") or "",
                "last_modified": item.get("last_modified") or item.get("updated_at") or "",
                "metadata": {
                    "size": item.get("size") or "",
                    "mime_type": item.get("mime_type") or item.get("mimetype") or "",
                    "resource_type": item.get("resource_type") or item.get("type") or "",
                },
            }
            try:
                resources.append(MCPResourceRecord.model_validate(normalized).model_dump(mode="json"))
            except Exception as exc:
                logger.warning("Skipping invalid live MCP resource item: %s", exc)
        return resources

    def _normalize_resource_info(self, payload: Dict[str, Any]) -> Dict[str, Any] | None:
        item = self._unwrap_single_item(payload)
        if item is None and isinstance(payload.get("result"), str):
            parsed_items = self._parse_resources_text(payload["result"])
            item = parsed_items[0] if parsed_items else None
        if not isinstance(item, dict):
            return None
        return {
            "id": item.get("id") or item.get("resource_id") or "",
            "title": item.get("title") or item.get("name") or "Untitled resource",
            "url": item.get("url") or item.get("download_url") or item.get("uri") or "",
            "format": (item.get("format") or item.get("filetype") or "").lower(),
            "description": item.get("description") or "",
            "last_modified": item.get("last_modified") or item.get("updated_at") or "",
            "metadata": {
                "size": item.get("size") or "",
                "mime_type": item.get("mime_type") or item.get("mimetype") or "",
                "resource_type": item.get("resource_type") or item.get("type") or "",
            },
        }

    def _extract_items(self, payload: Dict[str, Any]) -> List[Any]:
        for key in ("results", "datasets", "resources", "items", "data"):
            value = payload.get(key)
            if isinstance(value, list):
                return value
        return []

    def _unwrap_single_item(self, payload: Dict[str, Any]) -> Dict[str, Any] | None:
        if "dataset" in payload and isinstance(payload["dataset"], dict):
            return payload["dataset"]
        if "resource" in payload and isinstance(payload["resource"], dict):
            return payload["resource"]
        if "data" in payload and isinstance(payload["data"], dict):
            return payload["data"]
        if payload and "result" not in payload and "text" not in payload and all(not isinstance(value, list) for value in payload.values()):
            return payload
        return None

    def _organization_name(self, item: Dict[str, Any]) -> str:
        organization = item.get("organization")
        if isinstance(organization, dict):
            return str(organization.get("name", ""))
        if isinstance(organization, str):
            return organization
        owner = item.get("owner")
        if isinstance(owner, dict):
            return str(owner.get("name", ""))
        return ""

    def _parse_search_result_text(self, text: str) -> List[Dict[str, Any]]:
        pattern = re.compile(
            r"(?ms)^\d+\.\s(?P<title>.+?)\n"
            r"\s+ID:\s(?P<id>[^\n]+)\n"
            r"\s+Organization:\s(?P<organization>[^\n]+)\n"
            r"(?:\s+Tags:\s(?P<tags>[^\n]+)\n)?"
            r"\s+Resources:\s(?P<resources>[^\n]+)\n"
            r"\s+URL:\s(?P<url>\S+)"
        )
        items: List[Dict[str, Any]] = []
        for match in pattern.finditer(text):
            items.append(
                {
                    "id": match.group("id").strip(),
                    "title": match.group("title").strip(),
                    "description": "",
                    "url": match.group("url").strip(),
                    "organization": match.group("organization").strip(),
                    "tags": self._split_csv_like(match.group("tags")),
                    "resource_count": match.group("resources").strip(),
                }
            )
        return items

    def _parse_dataset_info_text(self, text: str) -> Dict[str, Any]:
        title = self._match_group(text, r"Dataset Information:\s(.+)")
        dataset_id = self._match_group(text, r"ID:\s([^\n]+)")
        url = self._match_group(text, r"URL:\s(\S+)")
        description = self._match_group(text, r"Full description:\s(.+?)(?:\n\n[A-Z][^\n]*:|\n\n##|\Z)", re.DOTALL)
        organization = self._match_group(text, r"Organization:\s([^\n]+)")
        license_name = self._match_group(text, r"License:\s([^\n]+)")
        last_update = self._match_group(text, r"Last updated:\s([^\n]+)")
        tags = self._split_csv_like(self._match_group(text, r"Tags:\s([^\n]+)"))
        return {
            "id": dataset_id.strip(),
            "title": title.strip() or "Untitled dataset",
            "description": description.strip(),
            "url": url.strip(),
            "organization": organization.strip(),
            "tags": tags,
            "license": license_name.strip(),
            "last_update": last_update.strip(),
        }

    def _parse_resources_text(self, text: str) -> List[Dict[str, Any]]:
        pattern = re.compile(
            r"(?ms)^\d+\.\s(?P<title>.+?)\n"
            r"\s+Resource ID:\s(?P<id>[^\n]+)\n"
            r"\s+Format:\s(?P<format>[^\n]+)\n"
            r"(?:\s+Size:\s(?P<size>[^\n]+)\n)?"
            r"(?:\s+MIME type:\s(?P<mime>[^\n]+)\n)?"
            r"(?:\s+Type:\s(?P<type>[^\n]+)\n)?"
            r"\s+URL:\s(?P<url>\S+)"
        )
        items: List[Dict[str, Any]] = []
        for match in pattern.finditer(text):
            items.append(
                {
                    "id": match.group("id").strip(),
                    "title": match.group("title").strip(),
                    "url": match.group("url").strip(),
                    "format": match.group("format").strip().lower(),
                    "description": "",
                    "last_modified": "",
                    "size": (match.group("size") or "").strip(),
                    "mime_type": (match.group("mime") or "").strip(),
                    "resource_type": (match.group("type") or "").strip(),
                }
            )
        return items

    def _match_group(self, text: str, pattern: str, flags: int = 0) -> str:
        match = re.search(pattern, text, flags)
        return match.group(1) if match else ""

    def _split_csv_like(self, value: str | None) -> List[str]:
        if not value:
            return []
        return [item.strip() for item in value.split(",") if item.strip()]

    def _parse_query_resource_data_text(self, text: str) -> ResourceSample:
        if not text or "Data (" not in text:
            return ResourceSample()

        rows: List[Dict[str, Any]] = []
        current_row: Dict[str, Any] | None = None
        in_data_section = False
        total_rows = self._match_group(text, r"Total rows \(Tabular API\):\s(\d+)")
        columns_raw = self._match_group(text, r"Columns:\s(.+)")
        for raw_line in text.splitlines():
            line = raw_line.rstrip()
            if line.startswith("Data ("):
                in_data_section = True
                continue
            if not in_data_section:
                continue
            if re.match(r"^\s*Row\s+\d+:", line):
                if current_row:
                    rows.append(current_row)
                current_row = {}
                continue
            if current_row is not None and line.startswith("    ") and ":" in line:
                key, value = line.strip().split(":", 1)
                current_row[key.strip()] = self._coerce_text_value(value.strip())
        if current_row:
            rows.append(current_row)
        return ResourceSample(
            rows=self._normalize_rows(rows),
            total_rows=int(total_rows) if total_rows else None,
            columns=[column.strip() for column in columns_raw.split(",")] if columns_raw else [],
            row_count_sampled=len(rows),
        )

    async def _resolve_resource_url(self, resource: MCPResourceRecord) -> str:
        resolved_url = str(resource.url)
        try:
            resource_payload = await self._call_tool("get_resource_info", {"resource_id": resource.id})
            normalized_resource = self._normalize_resource_info(resource_payload)
            if normalized_resource is not None:
                resolved_url = normalized_resource.get("url", resolved_url)
        except Exception as exc:
            logger.warning("Resource info lookup failed for %s: %s", resource.id, exc)
        return resolved_url

    async def _download_resource_text(self, url: str) -> tuple[str, int] | None:
        try:
            async with httpx.AsyncClient(timeout=settings.http_timeout_seconds, follow_redirects=True) as client:
                async with client.stream("GET", url) as response:
                    response.raise_for_status()
                    chunks: list[bytes] = []
                    total_bytes = 0
                    async for chunk in response.aiter_bytes():
                        if not chunk:
                            continue
                        chunks.append(chunk)
                        total_bytes += len(chunk)
                        if total_bytes > settings.max_full_resource_bytes:
                            return b"".decode("utf-8"), total_bytes
        except httpx.HTTPError as exc:
            logger.error("Full resource download failed for %s: %s", url, exc)
            return None

        payload = b"".join(chunks)
        return payload.decode("utf-8", errors="replace"), len(payload)

    def _coerce_text_value(self, value: str) -> Any:
        if value == "":
            return ""
        if value in {"True", "False"}:
            return value == "True"
        if re.fullmatch(r"-?\d+", value):
            try:
                return int(value)
            except ValueError:
                return value
        if re.fullmatch(r"-?\d+\.\d+", value):
            try:
                return float(value)
            except ValueError:
                return value
        if value.startswith("[") and value.endswith("]"):
            try:
                return json.loads(value)
            except json.JSONDecodeError:
                return value
        return value

    def _search_mock_data(self, queries: List[str]) -> List[Dict[str, Any]]:
        query_text = " ".join(queries).lower()
        if any(keyword in query_text for keyword in ["eau", "water", "qualite"]):
            return self._mock_catalog()["water"]
        if any(keyword in query_text for keyword in ["transport", "lyon", "retard", "mobilite"]):
            return self._mock_catalog()["transport"]
        return self._mock_catalog()["ev"]

    def _mock_catalog(self) -> Dict[str, List[Dict[str, Any]]]:
        return {
            "ev": [
                {
                    "id": "5448d3e0c751df01f85d0572",
                    "title": "Fichier consolidé des Bornes de Recharge pour Véhicules Électriques (IRVE)",
                    "description": "Base de données consolidée des infrastructures de recharge publique pour véhicules électriques.",
                    "url": "https://www.data.gouv.fr/fr/datasets/fichier-consolide-des-bornes-de-recharge-pour-vehicules-electriques-irve/",
                    "organization": "Ministère de la Transition écologique",
                    "metadata": {"topic": "electric-mobility", "source": "mock"},
                },
                {
                    "id": "60a3927b271101918a1a3e81",
                    "title": "Immatriculations des véhicules électriques",
                    "description": "Données sur les immatriculations de véhicules électriques et hybrides par région.",
                    "url": "https://www.data.gouv.fr/fr/datasets/immatriculations-vehicules-electriques/",
                    "organization": "Ministère de l'Intérieur",
                    "metadata": {"topic": "electric-mobility", "source": "mock"},
                },
            ],
            "water": [
                {
                    "id": "fr-water-001",
                    "title": "Qualité de l'eau potable - Paris",
                    "description": "Résultats de contrôle sanitaire de l'eau potable distribuée à Paris.",
                    "url": "https://www.data.gouv.fr/fr/datasets/qualite-de-leau-potable/",
                    "organization": "Ministère de la Santé",
                    "metadata": {"topic": "water-quality", "source": "mock"},
                },
                {
                    "id": "fr-water-002",
                    "title": "Points de prélèvement eau potable en Île-de-France",
                    "description": "Localisation des points de prélèvement utilisés pour le suivi de la qualité de l'eau.",
                    "url": "https://www.data.gouv.fr/fr/datasets/points-de-prelevement-eau-potable/",
                    "organization": "Agence régionale de santé",
                    "metadata": {"topic": "water-quality", "source": "mock"},
                },
            ],
            "transport": [
                {
                    "id": "fr-mobility-001",
                    "title": "Ponctualité du réseau TCL à Lyon",
                    "description": "Indicateurs de ponctualité et incidents sur le réseau de transport lyonnais.",
                    "url": "https://www.data.gouv.fr/fr/datasets/ponctualite-du-reseau-tcl/",
                    "organization": "SYTRAL Mobilités",
                    "metadata": {"topic": "public-transport", "source": "mock"},
                },
                {
                    "id": "fr-mobility-002",
                    "title": "Horaires théoriques du réseau TCL",
                    "description": "Horaires et structure GTFS du réseau de transport public de Lyon.",
                    "url": "https://www.data.gouv.fr/fr/datasets/horaires-theoriques-du-reseau-tcl/",
                    "organization": "SYTRAL Mobilités",
                    "metadata": {"topic": "public-transport", "source": "mock"},
                },
            ],
        }

    def _mock_dataset_details(self) -> Dict[str, Dict[str, Any]]:
        return {
            "5448d3e0c751df01f85d0572": {
                "id": "5448d3e0c751df01f85d0572",
                "title": "Fichier consolidé des Bornes de Recharge pour Véhicules Électriques (IRVE)",
                "description": "Base consolidée des infrastructures de recharge publique pour véhicules électriques.",
                "url": "https://www.data.gouv.fr/fr/datasets/fichier-consolide-des-bornes-de-recharge-pour-vehicules-electriques-irve/",
                "organization": "Ministère de la Transition écologique",
                "metadata": {"topic": "electric-mobility", "source": "mock"},
                "resources": [
                    {
                        "id": "irve-resource-001",
                        "title": "IRVE national summary",
                        "url": "https://www.data.gouv.fr/fr/datasets/r/irve-national-summary.csv",
                        "format": "csv",
                        "description": "Mock national summary for EV charging stations.",
                        "last_modified": "2026-02-28",
                    }
                ],
            },
            "60a3927b271101918a1a3e81": {
                "id": "60a3927b271101918a1a3e81",
                "title": "Immatriculations des véhicules électriques",
                "description": "Immatriculations de véhicules électriques et hybrides par région.",
                "url": "https://www.data.gouv.fr/fr/datasets/immatriculations-vehicules-electriques/",
                "organization": "Ministère de l'Intérieur",
                "metadata": {"topic": "electric-mobility", "source": "mock"},
                "resources": [
                    {
                        "id": "ev-registrations-001",
                        "title": "Electric vehicle registrations by region",
                        "url": "https://www.data.gouv.fr/fr/datasets/r/ev-registrations-by-region.csv",
                        "format": "csv",
                        "description": "Mock regional EV registration summary.",
                        "last_modified": "2026-01-31",
                    }
                ],
            },
            "fr-water-001": {
                "id": "fr-water-001",
                "title": "Qualité de l'eau potable - Paris",
                "description": "Contrôle sanitaire de l'eau potable distribuée à Paris.",
                "url": "https://www.data.gouv.fr/fr/datasets/qualite-de-leau-potable/",
                "organization": "Ministère de la Santé",
                "metadata": {"topic": "water-quality", "source": "mock"},
                "resources": [
                    {
                        "id": "water-quality-001",
                        "title": "Water quality controls - Paris",
                        "url": "https://www.data.gouv.fr/fr/datasets/r/water-quality-paris.json",
                        "format": "json",
                        "description": "Mock Paris water quality controls.",
                        "last_modified": "2026-02-15",
                    }
                ],
            },
            "fr-mobility-001": {
                "id": "fr-mobility-001",
                "title": "Ponctualité du réseau TCL à Lyon",
                "description": "Indicateurs de ponctualité et incidents sur le réseau lyonnais.",
                "url": "https://www.data.gouv.fr/fr/datasets/ponctualite-du-reseau-tcl/",
                "organization": "SYTRAL Mobilités",
                "metadata": {"topic": "public-transport", "source": "mock"},
                "resources": [
                    {
                        "id": "transport-punctuality-001",
                        "title": "TCL punctuality summary",
                        "url": "https://www.data.gouv.fr/fr/datasets/r/tcl-punctuality-summary.csv",
                        "format": "csv",
                        "description": "Mock punctuality summary for Lyon public transport.",
                        "last_modified": "2026-02-01",
                    }
                ],
            },
        }

    def _mock_resource_rows(self) -> Dict[str, List[Dict[str, Any]]]:
        return {
            "irve-resource-001": [
                {"region": "Ile-de-France", "department": "Paris", "stations": 420, "charge_points": 1250, "updated_at": "2026-02-28"},
                {"region": "Auvergne-Rhone-Alpes", "department": "Rhone", "stations": 310, "charge_points": 860, "updated_at": "2026-02-28"},
                {"region": "Nouvelle-Aquitaine", "department": "Gironde", "stations": 255, "charge_points": 640, "updated_at": "2026-02-28"},
            ],
            "ev-registrations-001": [
                {"region": "Ile-de-France", "registrations": 18420, "period": "2026-01-31"},
                {"region": "Auvergne-Rhone-Alpes", "registrations": 12980, "period": "2026-01-31"},
                {"region": "Occitanie", "registrations": 9420, "period": "2026-01-31"},
            ],
            "water-quality-001": [
                {"area": "Paris Centre", "samples": 34, "compliance_rate": 99.2, "control_date": "2026-02-10"},
                {"area": "Paris 15e", "samples": 28, "compliance_rate": 98.8, "control_date": "2026-02-12"},
                {"area": "Paris 19e", "samples": 31, "compliance_rate": 99.5, "control_date": "2026-02-15"},
            ],
            "transport-punctuality-001": [
                {"line": "Metro A", "punctuality_rate": 96.4, "incidents": 12, "period": "2026-01-01"},
                {"line": "Metro B", "punctuality_rate": 94.8, "incidents": 15, "period": "2026-01-01"},
                {"line": "Tram T1", "punctuality_rate": 92.7, "incidents": 21, "period": "2026-01-01"},
            ],
        }

    def _parse_resource_content(self, resource_format: str, text: str) -> List[Dict[str, Any]]:
        fmt = resource_format.lower()
        if fmt == "csv":
            return list(csv.DictReader(io.StringIO(text)))
        if fmt in {"json", "geojson"}:
            payload = json.loads(text)
            if isinstance(payload, list):
                return [row for row in payload if isinstance(row, dict)]
            if isinstance(payload, dict):
                if isinstance(payload.get("results"), list):
                    return [row for row in payload["results"] if isinstance(row, dict)]
                if payload.get("type") == "FeatureCollection" and isinstance(payload.get("features"), list):
                    return [feature.get("properties", {}) for feature in payload["features"] if isinstance(feature, dict)]
        return []

    def _normalize_rows(self, rows: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        normalized_rows: List[Dict[str, Any]] = []
        for row in rows:
            normalized_row: Dict[str, Any] = {}
            for key, value in row.items():
                normalized_key = str(key).strip() if key not in (None, "") else "unnamed_column"
                if normalized_key in normalized_row:
                    normalized_key = f"{normalized_key}_dup"
                normalized_row[normalized_key] = value
            normalized_rows.append(normalized_row)
        return normalized_rows

    def _extract_columns(self, rows: List[Dict[str, Any]]) -> List[str]:
        if not rows:
            return []
        columns: set[str] = set()
        for row in rows[:50]:
            columns.update(str(key) for key in row.keys())
        return sorted(columns)

    def _serialize_rows(self, resource_format: str, rows: List[Dict[str, Any]]) -> str:
        if not rows:
            return ""
        if resource_format.lower() == "csv":
            output = io.StringIO()
            writer = csv.DictWriter(output, fieldnames=list(rows[0].keys()))
            writer.writeheader()
            writer.writerows(rows)
            return output.getvalue()
        return json.dumps(rows, ensure_ascii=False, indent=2)

    def _parse_size_to_bytes(self, raw_size: str) -> int | None:
        if not raw_size:
            return None
        text = raw_size.strip().lower().replace(",", ".")
        match = re.match(r"^(\d+(?:\.\d+)?)\s*([kmgt]?i?b)?$", text)
        if not match:
            return None
        value = float(match.group(1))
        unit = (match.group(2) or "b").lower()
        multipliers = {
            "b": 1,
            "kb": 1_000,
            "kib": 1_024,
            "mb": 1_000_000,
            "mib": 1_048_576,
            "gb": 1_000_000_000,
            "gib": 1_073_741_824,
        }
        return int(value * multipliers.get(unit, 1))
