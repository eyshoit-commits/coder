# LLM Server (node-llama-cpp)

Der LLM-Server kapselt node-llama-cpp und bietet Token-kontrollierte Endpunkte:

- OpenAI-kompatible `/v1/*` APIs.
- Admin-Endpunkte zum Laden/Entladen von Modellen.
- Tokenverbrauch per Middleware + Redis Cache.
- Prometheus-kompatibles Metrics-Endpoint.
