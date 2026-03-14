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
        self.assertIn("/reports/{report_id}/{filename}", payload["endpoints"])

    def test_health_endpoint(self) -> None:
        response = self.client.get("/health")
        self.assertEqual(response.status_code, 200)
        payload = response.json()
        self.assertEqual(payload["status"], "ok")
        self.assertIn("mock_gemini", payload)
        self.assertIn("mock_mcp", payload)
        self.assertIn("vertex_code_execution_enabled", payload)

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
        self.assertIn("analysis_summary", payload)
        self.assertIn("key_findings", payload)
        self.assertIn("data_coverage", payload)
        self.assertIn("analysis_engine", payload)
        self.assertIn("dataset_columns", payload)
        self.assertIn("descriptive_statistics", payload)
        self.assertIn("regressions", payload)
        self.assertIn("charts", payload)
        self.assertIn("used_resources", payload)
        self.assertIn("report_artifacts", payload)
        self.assertGreaterEqual(len(payload["selected_sources"]), 1)
        self.assertGreaterEqual(len(payload["key_findings"]), 1)
        self.assertGreaterEqual(len(payload["used_resources"]), 1)
        self.assertGreaterEqual(len(payload["report_artifacts"]), 2)
        self.assertGreaterEqual(len(payload["dataset_columns"]), 1)
        self.assertIn(payload["analysis_engine"], {"heuristic-local", "vertex-code-execution"})
        self.assertEqual(payload["selected_sources"][0]["title"], payload["used_resources"][0]["dataset_title"])
        self.assertEqual({artifact["format"] for artifact in payload["report_artifacts"]}, {"pdf", "xlsx"})

        for artifact in payload["report_artifacts"]:
            report_response = self.client.get(artifact["download_url"])
            self.assertEqual(report_response.status_code, 200)
            if artifact["format"] == "pdf":
                self.assertEqual(report_response.headers["content-type"], "application/pdf")
            if artifact["format"] == "xlsx":
                self.assertEqual(
                    report_response.headers["content-type"],
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                )


if __name__ == "__main__":
    unittest.main()
