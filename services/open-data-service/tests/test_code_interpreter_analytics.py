import unittest

from app.schemas import ResourceTable
from app.services.code_interpreter_analytics import CodeInterpreterAnalyticsService


class CodeInterpreterAnalyticsServiceTestCase(unittest.TestCase):
    def test_local_analytics_returns_stats_regressions_and_charts(self) -> None:
        service = CodeInterpreterAnalyticsService()
        table = ResourceTable(
            text=(
                "date,region,stations,charge_points\n"
                "2026-01-01,Ile-de-France,10,40\n"
                "2026-01-02,Ile-de-France,12,48\n"
                "2026-01-03,Occitanie,8,30\n"
                "2026-01-04,Occitanie,15,60\n"
            ),
            rows=[
                {"date": "2026-01-01", "region": "Ile-de-France", "stations": 10, "charge_points": 40},
                {"date": "2026-01-02", "region": "Ile-de-France", "stations": 12, "charge_points": 48},
                {"date": "2026-01-03", "region": "Occitanie", "stations": 8, "charge_points": 30},
                {"date": "2026-01-04", "region": "Occitanie", "stations": 15, "charge_points": 60},
            ],
            total_rows=4,
            columns=["date", "region", "stations", "charge_points"],
            row_count_analyzed=4,
            content_bytes=128,
            full_download_used=True,
        )

        result = service.analyze(
            query="What are the latest official datasets about electric vehicle charging stations in France?",
            dataset_title="Mock IRVE",
            resource_title="mock.csv",
            resource_format="csv",
            table=table,
        )

        self.assertEqual(result.analysis_engine, "heuristic-local")
        self.assertGreaterEqual(len(result.descriptive_statistics), 2)
        self.assertGreaterEqual(len(result.regressions), 1)
        self.assertGreaterEqual(len(result.charts), 1)
        self.assertIn("stations", result.columns)


if __name__ == "__main__":
    unittest.main()
