# CyberDevStudio API Übersicht

Die detaillierte API-Spezifikation folgt dem JSON-RPC-Ansatz für Entwicklungsoperationen und stellt RESTful Endpunkte für Authentifizierung sowie LLM-Steuerung bereit. Dieses Dokument beschreibt die Zielstruktur und dient als Grundlage für die spätere Spezifikation.

## JSON-RPC Namespaces

- `project.*` – Projekt- und Dateiverwaltung.
- `sandbox.*` – Ausführung und Ressourcenmanagement.
- `agent.*` – LLM-Agenten, Prompt-Pipelines und Tools.
- `admin.*` – Admin-spezifische Operationen (z.B. Token-Adjustments).

Jede Methode erhält eine JSON-Schema Definition in `schemas/rpc` und wird über das Gateway (`apps/api`) bereitgestellt.

## Authentifizierung

- Benutzer melden sich mit Username/Passwort im Auth-Service an.
- JWT Tokens sichern API-Aufrufe; API-Keys für Dienst-zu-Dienst Verkehr.
- LLM-Aufrufe benötigen Header `X-Cyber-Token` für Budgetnachweis.

## LLM Server Endpunkte (node-llama-cpp)

| Methode | Pfad | Beschreibung |
| ------- | ---- | ------------ |
| `POST` | `/v1/chat/completions` | Chat-Completion API (OpenAI-kompatibel) |
| `POST` | `/v1/completions` | Prompt Completion |
| `POST` | `/v1/embeddings` | Embedding Berechnung |
| `POST` | `/admin/load` | Modell aus `/models` laden |
| `POST` | `/admin/unload` | Aktives Modell entladen |
| `GET` | `/admin/status` | Systemstatus (RAM, Tokens, Threads) |
| `GET` | `/admin/models` | Verfügbare GGUF-Modelle |
| `GET` | `/metrics` | Prometheus-kompatible Metriken |

## WebSocket Streams

- `wss://api:6813/agent/chat` – Streaming Antworten der Agenten.
- `wss://api:6813/sandbox/logs` – Live-Logs von Ausführungen.
- `wss://api:6813/admin/events` – Modell- und User-Events.

## Roadmap

1. Definition der JSON-Schemas für Kernaktionen.
2. Implementierung der Auth-Middleware mit JWT + API-Key Prüfung.
3. Aufbau des Telemetrie-Pipelines (OTEL + Prometheus).
4. Dokumentation der Fehlercodes und Ratenlimits.

## Implementierte JSON-RPC Methoden

### `fs.write`
- **Route:** `POST /rpc`
- **Beschreibung:** Schreibt Dateien relativ zum Workspace (UTF-8 oder Base64 Inhalt).
- **Antwort:** `{ "result": { "path": "<string>", "bytes": <number> } }`
- **Fehlercodes:**
  - `-32602` – Ungültige Parameter (z.B. absolute Pfade, Traversal, zu große Dateien).
  - `-32000` – Interner Dateisystemfehler.

### `run.exec`
- **Route:** `POST /rpc`
- **Beschreibung:** Führt zugelassene Kommandos innerhalb des Sandbox-Workspaces aus.
- **Antwort:**
  ```json
  {
    "result": {
      "status": { "success": true, "code": 0 },
      "stdout": "…",
      "stderr": "…",
      "duration_ms": 42
    }
  }
  ```
- **Fehlercodes:**
  - `-32010` – Kommando nicht erlaubt.
  - `-32011` – Timeout überschritten.
  - `-32012` – Ausgabebegrenzung überschritten.
  - `-32000` – Interner Ausführungsfehler (I/O, fehlende Streams, etc.).

Weitere Methoden folgen in späteren Iterationen.
