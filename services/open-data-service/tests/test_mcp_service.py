import unittest

from app.services.mcp_service import MCPService


class MCPServiceParsingTestCase(unittest.TestCase):
    def test_parse_semicolon_csv_into_multiple_columns(self) -> None:
        service = MCPService()
        content = (
            "\ufeffAnnée;Prélèvements eau potable;Total prélèvements millions m3\n"
            "2024;186000000;256.0\n"
            "2025;193500000;265.4\n"
        )

        rows = service._parse_resource_content("csv", content)

        self.assertEqual(len(rows), 2)
        self.assertEqual(list(rows[0].keys()), ["Année", "Prélèvements eau potable", "Total prélèvements millions m3"])
        self.assertEqual(rows[0]["Année"], 2024)
        self.assertEqual(rows[1]["Total prélèvements millions m3"], 265.4)


if __name__ == "__main__":
    unittest.main()
