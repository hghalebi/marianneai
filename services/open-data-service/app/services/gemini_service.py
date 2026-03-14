import json
from typing import Type, TypeVar

from pydantic import BaseModel

try:
    from google import genai
    from google.genai import types
except ImportError:  # pragma: no cover
    genai = None
    types = None

from app.config import settings
from app.utils.logger import logger

T = TypeVar("T", bound=BaseModel)


class GeminiService:
    """Thin wrapper around the Google GenAI SDK with a deterministic fallback."""

    def __init__(self) -> None:
        self.use_mock = settings.use_mock_gemini
        self.model_name = settings.gemini_model
        self.client = None

        if self.use_mock:
            return
        if not settings.gemini_api_key:
            logger.warning("GEMINI_API_KEY is missing. Falling back to mock mode.")
            self.use_mock = True
            return
        if genai is None or types is None:
            logger.warning("google-genai is not installed. Falling back to mock mode.")
            self.use_mock = True
            return

        self.client = genai.Client(api_key=settings.gemini_api_key)

    def generate_structured(self, prompt: str, response_model: Type[T], mock_response: T) -> T:
        if self.use_mock or self.client is None:
            logger.info("Mocking Gemini response for %s", response_model.__name__)
            return mock_response

        try:
            response = self.client.models.generate_content(
                model=self.model_name,
                contents=prompt,
                config=types.GenerateContentConfig(
                    response_mime_type="application/json",
                    response_schema=response_model,
                    temperature=0.2,
                ),
            )
            if getattr(response, "parsed", None):
                return response_model.model_validate(response.parsed)

            payload = getattr(response, "text", "")
            if not payload:
                raise ValueError("Gemini returned an empty response.")
            return response_model.model_validate(json.loads(payload))
        except Exception as exc:  # pragma: no cover
            logger.error("Gemini API error: %s. Falling back to mock mode.", exc)
            return mock_response
