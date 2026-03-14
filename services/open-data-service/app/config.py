from pathlib import Path

from pydantic_settings import BaseSettings, SettingsConfigDict

BASE_DIR = Path(__file__).resolve().parent.parent.parent.parent
SHARED_DIR = BASE_DIR / "shared"
SERVICE_DIR = BASE_DIR / "services" / "open-data-service"


class Settings(BaseSettings):
    app_name: str = "DataGouv Alive API"
    app_env: str = "development"
    app_port: int = 8000
    log_level: str = "INFO"
    gemini_api_key: str = ""
    gemini_model: str = "gemini-2.5-flash"
    use_mock_gemini: bool = True
    use_mock_mcp: bool = False
    mcp_server_url: str = "https://mcp.data.gouv.fr/mcp"
    mcp_search_path: str = "/tools/datagouv_search"
    http_timeout_seconds: float = 20.0
    enable_full_resource_download: bool = True
    max_full_resource_bytes: int = 8_000_000
    max_full_resource_rows: int = 25_000
    enable_vertex_code_execution: bool = False
    google_cloud_project: str = ""
    google_cloud_location: str = "europe-west1"
    vertex_code_execution_model: str = "gemini-2.5-flash"
    max_vertex_buffer_chars: int = 200_000
    cors_allow_origins: str = "*"
    shared_dir: Path = SHARED_DIR
    service_dir: Path = SERVICE_DIR
    reports_dir: Path = SERVICE_DIR / "generated_reports"

    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        extra="ignore",
    )

    @property
    def cors_origins(self) -> list[str]:
        if self.cors_allow_origins.strip() == "*":
            return ["*"]
        return [origin.strip() for origin in self.cors_allow_origins.split(",") if origin.strip()]


settings = Settings()
