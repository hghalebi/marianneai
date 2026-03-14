import json
from pathlib import Path
from uuid import uuid4
from xml.sax.saxutils import escape
import zipfile

from app.config import settings
from app.schemas import AnalysisResult, ReportArtifact, Source


class ReportService:
    """Generate lightweight PDF and XLSX reports without external dependencies."""

    def __init__(self) -> None:
        self.base_dir = settings.reports_dir
        self.base_dir.mkdir(parents=True, exist_ok=True)

    def create_reports(
        self,
        user_query: str,
        selected_sources: list[Source],
        analysis: AnalysisResult,
        answer: str,
        limitations: list[str],
    ) -> list[ReportArtifact]:
        report_id = uuid4().hex
        report_dir = self.base_dir / report_id
        report_dir.mkdir(parents=True, exist_ok=True)

        pdf_name = "analysis-report.pdf"
        xlsx_name = "analysis-report.xlsx"

        self._build_pdf(report_dir / pdf_name, user_query, selected_sources, analysis, answer, limitations)
        self._build_xlsx(report_dir / xlsx_name, user_query, selected_sources, analysis, answer, limitations)

        return [
            ReportArtifact(
                report_id=report_id,
                format="pdf",
                filename=pdf_name,
                download_url=f"/reports/{report_id}/{pdf_name}",
                content_type="application/pdf",
            ),
            ReportArtifact(
                report_id=report_id,
                format="xlsx",
                filename=xlsx_name,
                download_url=f"/reports/{report_id}/{xlsx_name}",
                content_type="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
        ]

    def resolve_report_path(self, report_id: str, filename: str) -> Path:
        path = (self.base_dir / report_id / filename).resolve()
        if not str(path).startswith(str(self.base_dir.resolve())):
            raise ValueError("Invalid report path.")
        return path

    def _build_pdf(
        self,
        path: Path,
        user_query: str,
        selected_sources: list[Source],
        analysis: AnalysisResult,
        answer: str,
        limitations: list[str],
    ) -> None:
        lines = [
            "DataGouv Alive Expert Report",
            "",
            f"User query: {user_query}",
            "",
            f"Analysis engine: {analysis.analysis_engine}",
            "",
            f"Executive answer: {answer}",
            "",
            f"Analysis summary: {analysis.analysis_summary}",
            "",
            "Key findings:",
        ]
        lines.extend(f"- {finding}" for finding in analysis.key_findings)
        if analysis.descriptive_statistics:
            lines.extend(["", "Descriptive statistics:"])
            lines.extend(
                f"- {item.column}: mean={item.mean}, median={item.median}, min={item.min}, max={item.max}"
                for item in analysis.descriptive_statistics[:4]
            )
        if analysis.regressions:
            lines.extend(["", "Linear regressions:"])
            lines.extend(
                f"- {item.feature_y} vs {item.feature_x}: slope={item.slope}, r2={item.r_squared}"
                for item in analysis.regressions[:3]
            )
        if analysis.charts:
            lines.extend(["", "Charts prepared for frontend:"])
            lines.extend(f"- {item.title} ({item.chart_type})" for item in analysis.charts[:3])
        lines.extend(["", "Selected sources:"])
        lines.extend(f"- {source.title} | {source.url}" for source in selected_sources)
        lines.extend(["", "Data coverage:", analysis.data_coverage or "No coverage information available.", "", "Limitations:"])
        lines.extend(f"- {limitation}" for limitation in limitations)

        stream_lines = ["BT", "/F1 11 Tf", "50 790 Td", "14 TL"]
        first_text = True
        for line in lines[:45]:
            safe = line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
            if first_text:
                stream_lines.append(f"({safe}) Tj")
                first_text = False
            else:
                stream_lines.append("T*")
                stream_lines.append(f"({safe}) Tj")
        stream_lines.append("ET")
        stream = "\n".join(stream_lines).encode("latin-1", errors="replace")

        objects = [
            b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj",
            b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj",
            b"3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj",
            f"4 0 obj << /Length {len(stream)} >> stream\n".encode("latin-1") + stream + b"\nendstream endobj",
            b"5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj",
        ]

        pdf = bytearray(b"%PDF-1.4\n")
        offsets = [0]
        for obj in objects:
            offsets.append(len(pdf))
            pdf.extend(obj)
            pdf.extend(b"\n")
        xref_offset = len(pdf)
        pdf.extend(f"xref\n0 {len(offsets)}\n".encode("latin-1"))
        pdf.extend(b"0000000000 65535 f \n")
        for offset in offsets[1:]:
            pdf.extend(f"{offset:010d} 00000 n \n".encode("latin-1"))
        pdf.extend(
            f"trailer << /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF".encode("latin-1")
        )
        path.write_bytes(pdf)

    def _build_xlsx(
        self,
        path: Path,
        user_query: str,
        selected_sources: list[Source],
        analysis: AnalysisResult,
        answer: str,
        limitations: list[str],
    ) -> None:
        sheets = [
            (
                "Summary",
                [
                    ["User query", user_query],
                    ["Analysis engine", analysis.analysis_engine],
                    ["Executive answer", answer],
                    ["Analysis summary", analysis.analysis_summary],
                    ["Data coverage", analysis.data_coverage],
                    ["Dataset row count", str(analysis.dataset_row_count or "")],
                    ["Dataset columns", ", ".join(analysis.dataset_columns)],
                    [],
                    ["Key findings"],
                    *[[finding] for finding in analysis.key_findings],
                    [],
                    ["Limitations"],
                    *[[limitation] for limitation in limitations],
                ],
            ),
            (
                "Findings",
                [["Rank", "Finding"]]
                + [[str(index), finding] for index, finding in enumerate(analysis.key_findings, start=1)],
            ),
            (
                "Statistics",
                [["Column", "Non null count", "Mean", "Median", "Min", "Max", "Stddev"]]
                + [
                    [
                        item.column,
                        str(item.non_null_count),
                        self._stringify(item.mean),
                        self._stringify(item.median),
                        self._stringify(item.min),
                        self._stringify(item.max),
                        self._stringify(item.stddev),
                    ]
                    for item in analysis.descriptive_statistics
                ],
            ),
            (
                "Regressions",
                [["Feature X", "Feature Y", "Slope", "Intercept", "R squared", "Sample size"]]
                + [
                    [
                        item.feature_x,
                        item.feature_y,
                        str(item.slope),
                        str(item.intercept),
                        str(item.r_squared),
                        str(item.sample_size),
                    ]
                    for item in analysis.regressions
                ],
            ),
            (
                "Charts",
                [["Chart ID", "Title", "Type", "Description", "X key", "Y keys", "Data JSON"]]
                + [
                    [
                        item.chart_id,
                        item.title,
                        item.chart_type,
                        item.description,
                        item.x_key,
                        ", ".join(item.y_keys),
                        json.dumps(item.model_dump(mode="json")["data"], ensure_ascii=False),
                    ]
                    for item in analysis.charts
                ],
            ),
            (
                "Sources",
                [["Title", "URL", "Description", "Reason", "Confidence"]]
                + [
                    [
                        source.title,
                        str(source.url),
                        source.description,
                        source.reason_for_selection,
                        str(source.confidence_score),
                    ]
                    for source in selected_sources
                ],
            ),
            (
                "Resources",
                [["Dataset", "Resource", "Format", "URL", "Score", "Coverage"]]
                + [
                    [
                        resource_analysis.dataset_title,
                        resource_analysis.resource_title,
                        resource_analysis.format,
                        str(resource_analysis.resource_url),
                        str(resource_analysis.score),
                        resource_analysis.coverage,
                    ]
                    for resource_analysis in analysis.resource_analyses
                ],
            ),
        ]

        for index, resource_analysis in enumerate(analysis.resource_analyses[:2], start=1):
            headers = resource_analysis.sample.columns or (
                list(resource_analysis.sample.rows[0].keys()) if resource_analysis.sample.rows else []
            )
            rows = [["Dataset", resource_analysis.dataset_title], ["Resource", resource_analysis.resource_title], []]
            if headers:
                rows.append(headers)
                rows.extend([[str(row.get(header, "")) for header in headers] for row in resource_analysis.sample.rows])
            sheets.append((f"Sample{index}", rows))

        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("[Content_Types].xml", self._content_types_xml(len(sheets)))
            archive.writestr("_rels/.rels", self._root_rels_xml())
            archive.writestr("xl/workbook.xml", self._workbook_xml(sheets))
            archive.writestr("xl/_rels/workbook.xml.rels", self._workbook_rels_xml(len(sheets)))
            archive.writestr("xl/styles.xml", self._styles_xml())
            for index, (_, rows) in enumerate(sheets, start=1):
                archive.writestr(f"xl/worksheets/sheet{index}.xml", self._sheet_xml(rows))

    def _content_types_xml(self, sheet_count: int) -> str:
        overrides = "".join(
            f'<Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
            for index in range(1, sheet_count + 1)
        )
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
            f"{overrides}"
            "</Types>"
        )

    def _root_rels_xml(self) -> str:
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
            "</Relationships>"
        )

    def _workbook_xml(self, sheets: list[tuple[str, list[list[str]]]]) -> str:
        sheet_entries = "".join(
            f'<sheet name="{escape(name)}" sheetId="{index}" r:id="rId{index}"/>'
            for index, (name, _) in enumerate(sheets, start=1)
        )
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
            'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
            f"<sheets>{sheet_entries}</sheets>"
            "</workbook>"
        )

    def _workbook_rels_xml(self, sheet_count: int) -> str:
        sheet_rels = "".join(
            f'<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>'
            for index in range(1, sheet_count + 1)
        )
        styles_rel = f'<Relationship Id="rId{sheet_count + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f"{sheet_rels}{styles_rel}"
            "</Relationships>"
        )

    def _styles_xml(self) -> str:
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
            '<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>'
            '<fills count="1"><fill><patternFill patternType="none"/></fill></fills>'
            '<borders count="1"><border/></borders>'
            '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
            '<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>'
            "</styleSheet>"
        )

    def _sheet_xml(self, rows: list[list[str]]) -> str:
        row_xml = []
        for row_index, row in enumerate(rows, start=1):
            cell_xml = []
            for col_index, value in enumerate(row, start=1):
                cell_ref = f"{self._column_name(col_index)}{row_index}"
                cell_xml.append(
                    f'<c r="{cell_ref}" t="inlineStr"><is><t>{escape(str(value))}</t></is></c>'
                )
            row_xml.append(f'<row r="{row_index}">{"".join(cell_xml)}</row>')
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
            f'<sheetData>{"".join(row_xml)}</sheetData>'
            "</worksheet>"
        )

    def _column_name(self, index: int) -> str:
        result = ""
        while index:
            index, remainder = divmod(index - 1, 26)
            result = chr(65 + remainder) + result
        return result

    def _stringify(self, value: object) -> str:
        return "" if value is None else str(value)
