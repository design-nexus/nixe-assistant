# Factual search MCP integration

Upstream: https://github.com/AstroCorp/MCP-DuckDuckGo

Pinned revision: `0cd5c5b9e8a4994736f18ef816c601dddf5a8324`.

The external server is GPL-3.0-only, retains its upstream source and LICENSE in the installed checkout, and communicates with the assistant over stdio. `astro-bounds.patch` and `verify.mjs` are GPL-3.0-only. The patch adds structured, bounded results, article/main extraction, publication metadata, HTTP timeouts, cancellation signals, response-size limits, and challenge detection. It does not solve CAPTCHAs. `astro-package-lock.json` pins the dependencies used for the successful live check.

Install with `scripts/install-search.sh`, then enable with `omarchy-assistant search-provider duckduckgo` and restart the daemon. Existing configuration is updated through the comment-preserving Rust configuration writer. Other tools from external servers retain their configured permissions.

The installer runs small fixtures for response size, challenge handling, cancellation, and extraction. It does not run the upstream suite's live-network query matrix.
