# nitrum-fn

A functions service: deploy Rust (or any WASM) functions and run them inside AWS Nitro enclaves, so every invoke can come back with a cryptographic proof of what code produced it — verifiable by anyone, including a smart contract.

This is a **proof of concept**, not a production service.

Built on [Nitrum](https://github.com/matzapata/nitrum), which owns the enclave platform (EIF builds, TLS-in-enclave, attestation, fleet). This repo is the WASM host, function catalog, publish/invoke API, and CLI.

## Demo

A WASM function fetches a price from CoinGecko inside the enclave. The response is attested, a relayer posts it on-chain, and a Solidity contract verifies the attestation before trusting the price.

<video src="docs/assets/oracle-demo.mp4" controls width="100%"></video>

Walkthrough: [`examples/oracle`](examples/oracle/README.md).

## Why

Normal APIs ask you to trust that a server ran the code it claims to. nitrum-fn proves it instead: the response is signed by AWS Nitro hardware and bound to the exact `.wasm` bytes that ran, so you — or a smart contract — can check it without trusting the operator.

## Use cases

- **On-chain oracles / verifiable off-chain compute** — fetch data off-chain, attest to it, verify on-chain before acting on it.
- **Verifiable tool calls for AI agents** — give an agent a function whose output is provably unmodified.
- **Pay-per-call APIs** — CLI-first, wallet-based access (planned).

## Quickstart

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum-fn/main/scripts/install-nitrum-fn.sh | bash

nitrum-fn new                                             # scaffold ./hello-world
nitrum-fn deploy ./hello-world/.../hello_world.wasm --name hello-world
nitrum-fn invoke hello-world -d '{}'
```

Full walkthrough (write a function, deploy, invoke, verify): **[docs/usage.md](docs/usage.md)**.

## Learn more

| | |
| --- | --- |
| [docs/architecture.md](docs/architecture.md) | Platform, trust boundary, crates, publish/invoke pipelines |
| [docs/usage.md](docs/usage.md) | Write a function, config, deploy, invoke, on-chain verification |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Local dev setup, checks, staging e2e, releases |
| [infra/README.md](infra/README.md) | Terraform / cloud deploy |
| [examples/](examples) | [hello-world](examples/hello-world/README.md), [oracle](examples/oracle/README.md) |
