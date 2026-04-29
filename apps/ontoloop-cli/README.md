# OntoLoop CLI Frontend (Minimal Subset)

This package is the first reusable CLI frontend shell for OntoLoop.

Scope for D2:
- Import minimal structure inspired by `opencode` CLI/TUI:
- `cmd` helper
- `thread` command
- `attach` command
- TUI event contract surface
- Run local build without external dependencies

Design notes:
- Existing application frontend stays untouched.
- This package is additive and dedicated to terminal UX.
- Runtime calls are routed through `src/adapters/ontoloop-client.js`.
