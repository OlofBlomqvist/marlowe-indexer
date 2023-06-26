# Marlowe Sync (indexer)

An indexer for Marlowe smart contracts on the Cardano blockchain.
Exposes indexed data via graphql.

Current state of this repository: very early POC/Demo implementation

**NOT YET READY FOR PRODUCTION USE**

# What works?

- Sync against cardano node over TCP/IP.
- Indexing of contracts using the V1 and V1+Audited Marlowe validators.
- Basic GraphQL server exposing:
    - All indexed contracts
    - Marlowe State / datum (in json format)
    - Marlowe Redeemer, also in json format
    - Generic info about each tx involved in a contract
    - Very limited filtering

# How it works

- Communicates via Cardano Nodes using [Pallas](https://github.com/txpipe/pallas) by the [txpipe](https://github.com/txpipe) team.
- Decoding of Marlowe on-chain data via [Marlowe_Lang](https://github.com/OlofBlomqvist/marlowe_lang).
- GraphQL server using [Async-GraphQL](https://github.com/async-graphql/async-graphql) & [Warp](https://github.com/seanmonstar/warp)

# How to run?

1. Make sure you have Rust installed: https://www.rust-lang.org/tools/install
2. Clone the repository
3. Run the application with host:port network: 
    ```
    cargo run 192.168.1.122:3001 preprod # mainnet/preprod/preview 
    ```
4. Open localhost:8000 in a browser

![graphql](https://github.com/OlofBlomqvist/marlowe_indexer/blob/main/graphql.png)

# Planned features

*In no particular order*

- Sync via unix-sockets
- Configuration via file and cli args
- Customization of how to store marlowe data: JSON/CBOR etc
- Data Persistance (redis,mongodb,file-system,etc?)
- GraphQL subscriptions (with filtering)
- Stabilize structure of stored data (what to store about each contract and how)

# Known hacks/flaws

- Rollbacks are not at all something that has been given any attention yet and may be completely broken.

# Code quality

- Initially during POC, shortcuts will be taken..
- Will be improved greatly before V1 release.

# When V1 ?

This is a side-project, so possibly never..


