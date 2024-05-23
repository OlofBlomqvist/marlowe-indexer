![CI Build](https://github.com/olofblomqvist/marlowe-indexer/actions/workflows/rust.yml/badge.svg?branch=main)

⚠️ **This project is highly unstable and not recommended for use in production systems.**

# Marlowe Indexer (WIP)

This is an experimental indexer for Marlowe contracts on the Cardano blockchain, based on the Haskell implementation by IOG [here](https://github.com/input-output-hk/marlowe-cardano/).

### How it works

- Communicates via Cardano Nodes using [Pallas](https://github.com/txpipe/pallas) by the [txpipe](https://github.com/txpipe) team.
- Decoding of Marlowe on-chain data via [Marlowe_Lang](https://github.com/OlofBlomqvist/marlowe_lang).
- GraphQL server using [Async-GraphQL](https://github.com/async-graphql/async-graphql) & [Warp](https://github.com/seanmonstar/warp)

### How to run?

*Pre-built binaries will be provided at a later date.*

1. Make sure you have Rust installed: https://www.rust-lang.org/tools/install
2. Clone the repository
3. Run the application using one of the following ways: 
    ```bash
    
    # Connect to a local node (defaults to CARDANO_NODE_SOCKET_PATH)
    cargo run -- socket-sync --network=preprod

    # Connect to a local node with specified path
    cargo run -- socket-sync --network=preprod -- "\\.\pipe\cardano-node-preprod"

    # Connect to a random remote node
    cargo run -- tcp-sync --network=preprod

    # Connect to a specific remote node
    cargo run -- tcp-sync --network=preprod -- 192.168.1.122:3000

    ```
4. Open localhost:8000 in a browser

![graphql](https://github.com/OlofBlomqvist/marlowe_indexer/blob/main/graphql.png)

## Current features

- Sync against cardano node over TCP/IP, UnixSocket or named pipes.
- Indexing of contracts using the V1 and V1+Audited Marlowe validators.
- Persistance to disk
- Basic GraphQL server exposing:
    - All indexed contracts
    - Marlowe State / datum (in json format)
    - Marlowe Redeemer, also in json format
    - Generic info about each tx involved in a contract
    - Limited filtering
    - Pagination
    - Subscription for contract events

### Planned features

*In no particular order*

- Improve estimation sync
- Configuration via file
- Indexing of addresses and their contents
- Improved subscriptions and filters such as
  - Filter contracts based on Open Role Tokens
  - Filter contracts based on available withdrawals/payouts
- Oracle helper subscriptions and API's

---

# Known issues & TODO's
- Pagination using the before & after props do not work correctly anymore
- Tests need to be updated
 
