import unittest

from fastapi.testclient import TestClient

from app.main import app


class ApiTestCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.client = TestClient(app)

    def test_root_endpoint(self) -> None:
        response = self.client.get("/")
        self.assertEqual(response.status_code, 200)
        payload = response.json()
        self.assertEqual(payload["status"], "ok")
        self.assertIn("/query", payload["endpoints"])

    def test_health_endpoint(self) -> None:
        response = self.client.get("/health")
        self.assertEqual(response.status_code, 200)
        payload = response.json()
        self.assertEqual(payload["status"], "ok")
        self.assertIn("mock_gemini", payload)
        self.assertIn("mock_mcp", payload)

    def test_demo_scenarios_endpoint(self) -> None:
        response = self.client.get("/demo/scenarios")
        self.assertEqual(response.status_code, 200)
        payload = response.json()
        self.assertIn("scenarios", payload)
        self.assertGreaterEqual(len(payload["scenarios"]), 1)

    def test_query_endpoint(self) -> None:
        response = self.client.post(
            "/query",
            json={"query": "What official datasets are available for electric vehicle charging stations in France?"},
        )
        self.assertEqual(response.status_code, 200)
        payload = response.json()
        self.assertEqual(
            payload["user_query"],
            "What official datasets are available for electric vehicle charging stations in France?",
        )
        self.assertIn("selected_sources", payload)
        self.assertIn("answer", payload)
        self.assertIn("limitations", payload)
        self.assertIn("trace", payload)
        self.assertGreaterEqual(len(payload["selected_sources"]), 1)


if __name__ == "__main__":
    unittest.main()
