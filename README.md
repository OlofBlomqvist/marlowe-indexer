![CI Build](https://github.com/olofblomqvist/marlowe-indexer/actions/workflows/rust.yml/badge.svg?branch=main)

# Marlowe Sync (indexer)

An indexer for Marlowe smart contracts on the Cardano blockchain.
Exposes indexed data via graphql.

Current state of this repository: Early alpha/poc phase

**NOT YET READY FOR PRODUCTION USE**

# What works?

- Sync against cardano node over TCP/IP, UnixSocket or named pipes.
- Indexing of contracts using the V1 and V1+Audited Marlowe validators.
- Basic GraphQL server exposing:
    - All indexed contracts
    - Marlowe State / datum (in json format)
    - Marlowe Redeemer, also in json format
    - Generic info about each tx involved in a contract
    - Limited filtering
    - Pagination

# How it works

- Communicates via Cardano Nodes using [Pallas](https://github.com/txpipe/pallas) by the [txpipe](https://github.com/txpipe) team.
- Decoding of Marlowe on-chain data via [Marlowe_Lang](https://github.com/OlofBlomqvist/marlowe_lang).
- GraphQL server using [Async-GraphQL](https://github.com/async-graphql/async-graphql) & [Warp](https://github.com/seanmonstar/warp)

# How to run?

1. Make sure you have Rust installed: https://www.rust-lang.org/tools/install
2. Clone the repository
3. Run the application using one of the following ways: 
    ```powershell
    
    # Connect to a local node (defaults to CARDANO_NODE_SOCKET_PATH)
    cargo run socket-sync --network=preprod

    # Connect to a local node with specified path
    cargo run socket-sync --network=preprod -- "\\.\pipe\cardano-node-preprod"

    # Connect to a random remote node
    cargo run tcp-sync --network=preprod

    # Connect to a specific remote node
    cargo run tcp-sync --network=preprod -- 192.168.1.122:3000

    ```
4. Open localhost:8000 in a browser

![graphql](https://github.com/OlofBlomqvist/marlowe_indexer/blob/main/graphql.png)

# Planned features

*In no particular order*

- Configuration via file
- Customization of how to store marlowe data: JSON/CBOR etc
- Data Persistance (redis,mongodb,file-system,etc?)
- GraphQL subscriptions (with filtering)
- Stabilize structure of stored data (what to store about each contract and how)

# Code quality

- Initially during POC, shortcuts will be taken..
- Will be improved greatly before V1 release.
