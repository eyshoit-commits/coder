# Akzeptanzkriterien – CyberDevStudio

Diese Datei sammelt die akzeptanzrelevanten Prüfpunkte für CyberDevStudio. Die Kriterien werden pro Release-Zyklus erweitert.

## Kernfunktionen
- Benutzer können sich mit JWT anmelden und erhalten rollenspezifische Rechte.
- Projekte lassen sich im Studio anlegen, Dateien bearbeiten und in der Sandbox ausführen.
- LLM-Aufrufe werden über node-llama-cpp abgewickelt und auf Tokenverbrauch geprüft.
- Admins können Modelle laden/entladen und Nutzerbudgets verwalten.
- Telemetrie ist über `/metrics` sowie das Admin-Dashboard abrufbar.

## Tests
- Unit-Tests decken Sandbox-, Auth- und RPC-Module ab.
- End-to-End Tests prüfen `fs.write`, `run.exec` und `llm.chat`.
- Fehlerpfade (401, 403, 429, 500) werden simuliert und protokolliert.

## Nichtfunktionale Anforderungen
- Alle Services laufen auf nicht-standard Ports (siehe Docker Compose).
- Ressourcenlimits: CPU/Memory für Sandbox, Tokenlimits für Modelle.
- Sicherheit: API-Keys, TLS-Termination, Audit Logging.
