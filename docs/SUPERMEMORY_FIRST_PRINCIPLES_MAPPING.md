# Supermemory First-Principles Mapping (Stage 0)

This document extracts the minimal first-principles loop from `supermemory-main` and maps it to the current AutoLoop architecture without changing business logic.

## 1) Minimal Supermemory Loop

`Queued -> Extracting -> Chunking -> Embedding -> Indexing -> Done`

Core idea:

- Treat incoming content as a processing job.
- Convert raw content into semantic chunks.
- Convert chunks into vector-like semantic signals.
- Build atomic memories and memory relations (`updates`, `extends`, `derives`, `isLatest`).
- Persist document/chunk/memory/relations so retrieval is memory-first but evidence-grounded.

## 2) Strict Architecture Mapping

### `Knowledge Context Sources`

Maps to all source inputs before memory computation:

- plaza feed / org KB / replay scope / playbooks / org memory slice / private memory

Current AutoLoop anchors:

- `src/orchestration/knowledge_context.rs`
- `src/memory/mod.rs`
- `src/rag/*`

### `Ingestion / Processing Queue`

Maps to queued jobs that track content entering memory pipeline.

Target storage prefix:

- `memory:supermemory:queue:*`

### `Content Extraction + Chunking + Metadata Capture`

Maps to normalized text extraction, semantic chunk segmentation, and source/date/tags capture.

Target storage prefixes:

- `memory:supermemory:documents:*`
- `memory:supermemory:chunks:*`

### `Memory Generation Layer`

Maps to:

- atomic memories
- embeddings
- relationship construction (`updates`, `extends`, `derives`, `isLatest`)
- temporal grounding (`documentDate`, `eventDate`)

Target storage prefixes:

- `memory:supermemory:atomic:*`
- `memory:supermemory:embeddings:*`
- `memory:supermemory:relations:*`
- `memory:supermemory:temporal:*`

### `Memory Graph + Document Store`

Maps to co-existing persistence of:

- raw/extracted documents
- semantic chunks
- atomic memories
- relations and temporal metadata

### `User Profile Builder + Hybrid Search Engine`

Maps to:

- user profile synthesis (`static` + `dynamic`)
- memory-first retrieval, then source chunk evidence injection

Target storage prefixes:

- `memory:supermemory:profile:*`
- `memory:supermemory:context:*`

### `Context Assembly / Retrieval Output -> Knowledge Context Injector`

Maps to final context bundle produced per query/run:

- profile
- relevant memories
- source evidence refs
- document refs

This bundle is consumed by the orchestration context injection stage for downstream planning/execution.

## 3) Stage 0 Exit Criteria

Stage 0 is complete when:

- The six-step loop is explicitly extracted.
- Every step is mapped to an AutoLoop architecture node.
- Storage and retrieval boundaries are clear (`queue -> process -> memory graph/doc store -> hybrid retrieval -> context assembly`).