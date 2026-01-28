# Slipstream SDK Architecture

This document describes the internal architecture of the Slipstream Rust SDK, focusing on how it ensures low-latency, reliable connections to the global mesh.

## 1. Connection Lifecycle

When `SlipstreamClient::connect()` is called, the following process occurs:

### A. Worker Selection (Discovery Phase)
Unless a specific `endpoint` is provided in the config:
1.  The SDK initializes the `WorkerSelector`.
2.  It loads a list of available worker nodes (bootstrap list or cached).
3.  **Parallel Pinging**: The SDK sends concurrent HTTP `HEAD` requests to the `/health` endpoint of known workers.
4.  **Selection**: It calculates the RTT (Round Trip Time) for each and selects the worker with the lowest latency.
5.  **Region Preference**: If `region` is configured (e.g., "us-east"), it prioritizes workers in that region.

### B. Protocol Negotiation (Fallback Chain)
Once a target worker (IP/Host) is selected, the `FallbackChain` attempts to establish a connection using protocols in the following order:

1.  **QUIC (UDP)**:
    -   Target Port: `4433`
    -   Timeout: 2s
    -   Pros: Lowest latency, 0-RTT resumption, no head-of-line blocking.
    -   Cons: May be blocked by strict corporate firewalls (UDP).

2.  **gRPC (HTTP/2)**:
    -   Target Port: `10000`
    -   Timeout: 3s
    -   Pros: Multiplexing, strict typing, generally firewall-friendly.

3.  **WebSocket (TCP)**:
    -   Target Port: `9000` (`/ws`)
    -   Timeout: 3s
    -   Pros: Ubiquitous support, full-duplex.

4.  **HTTP (Polling)**:
    -   Target Port: `9000`
    -   Timeout: 5s
    -   Pros: Works everywhere.
    -   Cons: High latency (polling).

The first protocol to successfully handshake is used for the session.

### C. Health Monitoring
The `HealthMonitor` runs in a background Tokio task:
-   It periodically checks the connection status.
-   **Auto-Reconnect**: If the connection drops, it triggers the Discovery and Negotiation phases again transparently.

## 2. Updates & Streaming

The SDK maintains persistent streams for intelligence data if enabled:

-   **Leader Hints**: Pushed via QUIC streams or gRPC server-streaming.
-   **Tip Instructions**: Real-time updates on optimal Jito/Solana tips.

## 3. Class Structure

-   **`SlipstreamClient`**: The high-level facade. Thread-safe, clonable (uses `Arc`).
-   **`Transport` (Trait)**: Abstract interface implemented by `QuicTransport`, `GrpcTransport`, etc.
-   **`WorkerSelector`**: Manages latency stats and endpoint selection.
-   **`Config`**: Immutable configuration object.

## 4. Security

-   **TLS**: All connections (QUIC, gRPC, WSS, HTTPS) are encrypted using TLS 1.3.
-   **Authentication**: API keys are passed in headers (`x-api-key`) or initial handshake frames.
