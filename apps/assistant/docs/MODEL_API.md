# Local model API v1

Suite applications use the existing private Unix socket and correlated request/result envelopes. No HTTP listener, new service, account, or model download is introduced.

Send one JSON line with `type: request`, a unique `request_id`, and an `action` object. Ignore unrelated events and accept only the matching `type: result` envelope. The request deadline must be absolute. Existing commands and news completion behavior remain unchanged.

Actions:

- `model_capabilities`: returns `api_version: 1`, `generation`, `local`, `embedding`, `embedding_model`, `embedding_profile`, and `embedding_endpoint`.
- `model_generate`: takes `system`, `text`, optional `max_tokens` (1–4096), and optional `temperature` (0–2). Calls the resident local model with no tools, speech, conversation history, or web retrieval. Returns `{api_version, text, local}`. Concurrent voice/model turns report busy. Generation has a 120-second ceiling and a 1 MiB output limit.
- `model_configure_embeddings`: explicitly sets `enabled`, `endpoint`, and `model` in Assistant's configuration. The endpoint must use a literal loopback IP address; redirects, proxies, credentials, and cloud URLs are refused. Disabling it stops future embedding requests.
- `model_embed`: takes `input`, up to 32 strings/128 KiB total. Returns `{api_version, model, profile, vectors}`. Model/server selection remains Assistant-owned. Vector counts, dimensions, ordering, and finite numeric values are checked. No model is downloaded or started automatically.

Embeddings default to disabled with no endpoint or model. An explicitly configured local model server must accept a JSON POST containing `model`, string-array `input`, and `encoding_format: float`, and return `data` rows with `index` and `embedding`. This transport is independent of a particular inference engine; the provider boundary can add native backends later. Note applications must keep derived indexes rebuildable and invalidate them when the profile changes.

Notes uses this API only after opt-in. Generated edits are proposals and need visible diff approval. Note excerpts are untrusted data, never executable tool instructions. Model API calls do not grant permission to search or edit vault files; those permissions belong to the Notes control interface.

Installing an update does not restart an already running daemon. Older processes do not implement these actions; clients must fail gracefully until the user's normal restart/session transition. Verification uses protocol/URL fixtures and compilation, without model inference.
