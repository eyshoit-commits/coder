# API Gateway (Rust)

Dieses Verzeichnis beherbergt den JSON-RPC Gateway Service. Aktueller Stand:

- Axum-basierter HTTP Endpunkt `/rpc` für die Methoden `fs.write` und `run.exec`.
- Einfache Fehlerabbildung auf JSON-RPC Codes (`-3260x`, `-3201x`).
- Weiterleitung an das Sandbox-Crate für Dateisystem- und Prozessoperationen.
- Healthcheck unter `/health`.
- Umfangreiche Unit-/Integrationstests mit temporären Workspaces.

Geplante nächste Schritte:

- Verbindung zur PostgreSQL/PostgresML Datenbank.
- RPC Dispatch, Policy Checks, Token-Billing Hooks.
- Integration mit OpenTelemetry und Rate-Limiting.
- WebSocket Routen für Streaming-Ausgaben.
