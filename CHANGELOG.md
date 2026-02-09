# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Keep-alive & time sync**: Background ping loop (default 5s) measures RTT and synchronizes clocks with the server using NTP-style calculation.
- `ping()` method — send a single ping and get `PingResult { seq, rtt_ms, clock_offset_ms, server_time }`.
- `latency_ms()` — median one-way latency from sliding window of 10 samples.
- `clock_offset_ms()` — median clock offset between client and server.
- `server_time()` — current server time estimated from local clock + offset.
- `Config::builder().keepalive(bool)` — enable/disable keep-alive (default: true).
- `Config::builder().keepalive_interval(secs)` — ping interval (default: 5s).
- `PingResult` type exported from crate root.
- QUIC transport: binary ping/pong over bidi stream (StreamType 0x08).
- WebSocket transport: JSON-based ping/pong messages.
- HTTP transport: `GET /v1/ping` endpoint for time sync.
- **Free tier usage**: `get_free_tier_usage()` method in SDK.
- **Stream billing**: Leader hints, tip instructions, priority fees, latest blockhash, and latest slot streams are now billed (1 token per subscription, 1-hour reconnect grace period).
- **Latest blockhash stream**: `subscribe_latest_blockhash()` — blockhash updates every 2s.
- **Latest slot stream**: `subscribe_latest_slot()` — slot updates on every slot change (~400ms).
- **Priority tiers**: `Config::builder().tier("pro")` — set billing tier (free/standard/pro/enterprise).

- Initial SDK setup.
