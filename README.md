# hopr-api

[![Crates.io](https://img.shields.io/crates/v/hopr-api)](https://crates.io/crates/hopr-api)
[![docs.rs](https://img.shields.io/docsrs/hopr-api)](https://docs.rs/hopr-api)
[![Security](https://github.com/hoprnet/hopr-api/actions/workflows/checks-zizmor.yaml/badge.svg)](https://github.com/hoprnet/hopr-api/actions/workflows/checks-zizmor.yaml)
[![License](https://img.shields.io/crates/l/hopr-api)](https://github.com/hoprnet/hopr-api/blob/main/LICENSE)
[![MSRV](https://img.shields.io/crates/msrv/hopr-api)](https://github.com/hoprnet/hopr-api)
[![Crates.io Downloads](https://img.shields.io/crates/d/hopr-api)](https://crates.io/crates/hopr-api)

Common high-level API traits for the [HOPR protocol](https://hoprnet.org/).

This crate defines the public API surface as **traits only** — no concrete implementations.
Implementations live in the main [hoprnet](https://github.com/hoprnet/hoprnet) repository.

## Getting started

Choose the smallest feature set you need:

```toml
[dependencies]
hopr-api = { version = "4.0.0", features = ["chain", "network"] }
```

or enable the complete API surface:

```toml
[dependencies]
hopr-api = { version = "4.0.0", features = ["full"] }
```

Then import the traits your component implements:

```rust
use hopr_api::chain::{ChainReadAccountOperations, ChainValues};
use hopr_api::network::NetworkView;
```

## Modules

All modules are feature-gated:

| Feature               | Module          | Description                                            |
| --------------------- | --------------- | ------------------------------------------------------ |
| `chain`               | `chain`         | On-chain operation APIs (accounts, channels, tickets…) |
| `ct`                  | `ct`            | Cover traffic and probing API traits                   |
| `graph`               | `graph`         | Network graph topology, QoS, routing costs             |
| `network`             | `network`       | Network state, peer observations, connectivity         |
| `node`                | `node`          | High-level HOPR node API traits and state machine      |
| `node-session-client` | `node::session` | Session client for establishing HOPR sessions          |
| `node-session-server` | `node::session` | Session server for processing incoming sessions        |
| `tickets`             | `tickets`       | Winning ticket management and redemption               |
| `full`                | _all_           | Enables all of the above + `serde`                     |

## Usage

```toml
[dependencies]
hopr-api = { version = "1.7.0", features = ["full"] }
```

## Development

Requires [Nix](https://nixos.org/) with flakes enabled.

```bash
# Enter development shell
nix develop

# Build
cargo build

# Test
cargo test --lib

# Lint
cargo clippy

# Format
nix fmt
```

## License

[GPL-3.0-only](https://github.com/hoprnet/hopr-api/blob/main/LICENSE)

## Component operational state

Enable `component` (included in `full`) to use `ComponentState` and `AtomicComponentState`.
`Running` means the component can perform all its current responsibilities; `Degraded` means it can perform some; `Failed` means it cannot perform any.
States default to `Running` and are ordered by severity. Each implementation owns its transitions and recovery behavior.
The `serde` feature enables serialization and deserialization. Enable both `component` and `serde` to use `StateDump`, the optional asynchronous diagnostic interface.
Generic callers can collect an opaque owned Serde snapshot through `state_dump().await`; concrete snapshot models and collection logic remain inside each implementing crate.
The interface delegates through references, `Box` and `Arc`. Each implementation defines its snapshot schema and documents whether reads consume observations.
Components requiring caller-supplied context may return an opaque configured view implementing `StateDump`.
Operational state and diagnostic collection errors remain separate; timers, logging and deadlines are not part of the shared interface.
