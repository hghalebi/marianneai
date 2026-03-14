from typing import Any, Dict, List

import httpx

from app.config import settings
from app.schemas import MCPDatasetRecord
from app.utils.logger import logger


class MCPService:
    """Adapter for data.gouv search via MCP, with mock datasets for demos."""

    def __init__(self) -> None:
        self.use_mock = settings.use_mock_mcp
        self.mcp_url = settings.mcp_server_url

    async def search_data_gouv(self, queries: List[str]) -> List[Dict[str, Any]]:
        if self.use_mock:
            logger.info("Mocking MCP search for queries: %s", queries)
            return self._search_mock_data(queries)

        try:
            async with httpx.AsyncClient(timeout=settings.http_timeout_seconds) as client:
                response = await client.post(
                    f"{self.mcp_url.rstrip('/')}{settings.mcp_search_path}",
                    json={"queries": queries},
                )
                response.raise_for_status()
            payload = response.json()
        except httpx.HTTPError as exc:
            logger.error("MCP HTTP error: %s. Falling back to mock mode.", exc)
            return self._search_mock_data(queries)

        normalized_results: List[Dict[str, Any]] = []
        for item in payload.get("results", []):
            try:
                normalized_results.append(MCPDatasetRecord.model_validate(item).model_dump(mode="json"))
            except Exception as exc:
                logger.warning("Skipping invalid MCP item: %s", exc)

        return normalized_results or self._search_mock_data(queries)

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
                    "metadata": {"topic": "electric-mobility"},
                },
                {
                    "id": "60a3927b271101918a1a3e81",
                    "title": "Immatriculations des véhicules électriques",
                    "description": "Données sur les immatriculations de véhicules électriques et hybrides par région.",
                    "url": "https://www.data.gouv.fr/fr/datasets/immatriculations-vehicules-electriques/",
                    "organization": "Ministère de l'Intérieur",
                    "metadata": {"topic": "electric-mobility"},
                },
            ],
            "water": [
                {
                    "id": "fr-water-001",
                    "title": "Qualité de l'eau potable - Paris",
                    "description": "Résultats de contrôle sanitaire de l'eau potable distribuée à Paris.",
                    "url": "https://www.data.gouv.fr/fr/datasets/qualite-de-leau-potable/",
                    "organization": "Ministère de la Santé",
                    "metadata": {"topic": "water-quality"},
                },
                {
                    "id": "fr-water-002",
                    "title": "Points de prélèvement eau potable en Île-de-France",
                    "description": "Localisation des points de prélèvement utilisés pour le suivi de la qualité de l'eau.",
                    "url": "https://www.data.gouv.fr/fr/datasets/points-de-prelevement-eau-potable/",
                    "organization": "Agence régionale de santé",
                    "metadata": {"topic": "water-quality"},
                },
            ],
            "transport": [
                {
                    "id": "fr-mobility-001",
                    "title": "Ponctualité du réseau TCL à Lyon",
                    "description": "Indicateurs de ponctualité et incidents sur le réseau de transport lyonnais.",
                    "url": "https://www.data.gouv.fr/fr/datasets/ponctualite-du-reseau-tcl/",
                    "organization": "SYTRAL Mobilités",
                    "metadata": {"topic": "public-transport"},
                },
                {
                    "id": "fr-mobility-002",
                    "title": "Horaires théoriques du réseau TCL",
                    "description": "Horaires et structure GTFS du réseau de transport public de Lyon.",
                    "url": "https://www.data.gouv.fr/fr/datasets/horaires-theoriques-du-reseau-tcl/",
                    "organization": "SYTRAL Mobilités",
                    "metadata": {"topic": "public-transport"},
                },
            ],
        }
