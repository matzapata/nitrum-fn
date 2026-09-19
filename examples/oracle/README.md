# Oracle example (enclave guest + Foundry consumer)

A WASM function fetches a price from CoinGecko inside a nitrum-fn enclave. A relayer posts
the attested response on-chain; `NitrumOracle` verifies it (AWS Nitro PKI + PCR0 + content
hash + body hash) before storing the price.

<video src="../../docs/assets/oracle-demo.mp4" controls width="100%"></video>

```text
examples/oracle/
  e2e.sh         # staging demo: deploy → setEnclave → invoke/submit/read × 2
  enclave/       # CoinGecko guest (.wasm) — canonical JSON body
  contracts/     # Foundry: NitrumOracle, Deploy, SetEnclave, UpdatePrices
```

## Run it

```bash
# .env: PRIVATE_KEY, BASE_SEPOLIA_RPC_URL, ORACLE_ADDRESS
# optional NITRUM_FN_API_URL / NITRUM_FN_INVOKE_URL / NITRUM_FN_PCR0 (else terraform outputs)
set -a && source .env && set +a
./examples/oracle/e2e.sh
```

That builds the guest, deploys it, pins the enclave on-chain (`setEnclave`), takes an
attested invoke, submits it, and reads the price back — twice (cold cert cache, then warm).

## What gets verified

Trust is **not** a Nitrum public key. `NitrumOracle` checks:

1. **AWS Nitro PKI** — the COSE document is signed under the pinned AWS Nitro root CA ([`NitroValidator`](https://github.com/base/nitro-validator)).
2. **PCR0** — the enclave image measurement, pinned via `setEnclave`.
3. **Content hash** — the guest `.wasm` (`sha256(wasm)` — not AWS PCR1, a different measurement).
4. **Body hash** — the posted price bytes match what the enclave attested to.

Full step-by-step (deploy contracts, pin the enclave, invoke, submit, common gotchas):
[docs/usage.md#on-chain-verification](../../docs/usage.md#on-chain-verification).
