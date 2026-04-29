# AutoLoop CLI Specification

## Scope
`autocog` product-facing command surface for operations, knowledge delivery, and trigger ingress.

## Command Families
- `autocog knowledge ...`
- `autocog trigger ...`
- `autocog frontend ...`
- `autocog system ...`

## Knowledge Commands
- `autocog knowledge export --anchor-id <session> --type <graph|index|research|...>`
- `autocog knowledge batch-export --anchor-list <file> --type <graph|index|research|...>`
- `autocog knowledge replay-report --anchor-id <session> --snapshot-id <id>`

Batch list format:
- plain text, one session/anchor id per line
- empty lines and `# comment` lines are ignored

## Trigger Commands
- `autocog trigger set --anchor-id <session> --schedule <topic> --payload <json>`
- `autocog trigger list --anchor-id <session>`
- `autocog trigger run --anchor-id <session>`
- `autocog trigger webhook --anchor-id <session> --topic <event.topic> --payload <json> --actor <id> [--run-now]`
- `autocog trigger daemon --anchor-id <session> --schedule <interval-seconds>`
- `autocog trigger cancel --anchor-id <session>`

Webhook semantics:
- topic is normalized by runtime to `trigger:webhook:*`
- event is persisted before execution
- with `--run-now`, worker executes once immediately and returns combined report

## System Commands
- `autocog system export --session <id> [--trace-id <id>]`
- `autocog system query --session <id> [--trace-id <id>]`
- `autocog system replay-report --session <id> [--snapshot-id <id>]`

`system export` returns one product bundle:
- system status
- dashboard snapshot
- unified query view
- knowledge graph snapshot

## Frontend Commands
- `autocog frontend status --session <id>`
- `autocog frontend events --session <id> --format <pretty|json> --limit <n>`

Frontend command semantics:
- `status` exposes transport v2 event coverage (`ready/state_snapshot/assistant_delta/tool_started/tool_completed`) and bridge/runtime observable snapshots.
- `events` returns a tail view of session transport events for CLI-first debugging and replay explainability.
- Existing application frontend assets (for example `dashboard-ui/`) remain untouched; CLI frontend is an additional shell surface, not a replacement.

## Output Contract
- default output: JSON text to stdout
- optional `--output <path>` writes JSON payload to file
- errors keep JSON error envelope where possible
