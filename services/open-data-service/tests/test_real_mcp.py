import unittest

from app.config import settings
from app.services.mcp_service import MCPService


@unittest.skipIf(settings.use_mock_mcp, "Real MCP integration test is disabled in mock mode.")
class RealMCPIntegrationTestCase(unittest.IsolatedAsyncioTestCase):
    async def test_live_search_datasets(self) -> None:
        service = MCPService()
        results = await service.search_data_gouv(["bornes recharge vehicules electriques"])

        self.assertFalse(service.use_mock)
        self.assertGreaterEqual(len(results), 1)
        self.assertEqual(results[0]["metadata"]["source"], "mcp-live")
        self.assertIn("data.gouv.fr", results[0]["url"])


if __name__ == "__main__":
    unittest.main()
