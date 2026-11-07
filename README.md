# OpenFi on Stellar: Allowance State Machine and Settlement Router (prototype)

OpenFi lets treasury teams and AI agents pull funds from bank accounts under on-chain spending limits, with cleared funds settling on Stellar. This repository holds the first slice of the Stellar layer: a Soroban **custom-account** contract that enforces an agent's mandate. An agent holds a session key that can only move funds within a per-epoch cap, before an expiry, to allowlisted destinations, and only until the owner revokes it. Every check runs in `__check_auth`, so a transfer that breaks the mandate fails on-chain.

## Status

| Part | State |
|---|---|
| Allowance State Machine (cap per epoch, expiry, revoke, destination allowlist) | **Real**, deployed on testnet |
| Credential Registry check at mandate creation | **Stubbed**: an issuer-maintained allowlist (`set_verified`). The full Registry is a funded deliverable. |
| Settlement Router: releases USDC for a cleared bank debit, idempotent per payment ID | **Real**, deployed on testnet ([CDG3QJXN…KJCI](https://stellar.expert/explorer/testnet/contract/CDG3QJXN5CSDH7MXH5FXQVLG4O5GAYXJWCBHCTK3AMCN45QA3MWLKJCI)) |
| Bank-debit webhook listener (`scripts/settle-server.js`) | **Real**. Verified on testnet with a locally signed test webhook ([settlement tx](https://stellar.expert/explorer/testnet/tx/ec353e18bf9ca1f73f50247a4004f64bb212e6bced29c952cd79e11e93ea2ebc)). Run against the open banking sandbox: in progress. |
| Router authority | Prototype uses a single key; production uses a threshold multisig. No mandate or Registry check inside the router yet (funded work). |
| One token and one session key per mandate | Prototype limits. Multi-token and multi-key mandates are funded work. |

## Testnet evidence

Run of 2026-11-07 with Circle's testnet USDC. Every row is verifiable on the explorer; the two failed transactions show the contract's error code in their diagnostic events.

| Item | Link |
|---|---|
| Allowance account contract | [CCA3E57A…HIQAU](https://stellar.expert/explorer/testnet/contract/CCA3E57AE4TU3LDTKCREKXTGGJZVDOT2DXCXXSHMUGOL3EBD467HIQAU) |
| Token: USDC (testnet) | [CBIELTK6…DAMA](https://stellar.expert/explorer/testnet/contract/CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA) |
| Mandate #1 created ($50/day cap) | [0ca7dd16…](https://stellar.expert/explorer/testnet/tx/0ca7dd16b76aff3356702fb6c908665563f5473dadc96e0181b4e491a1b97de8) |
| $30 transfer accepted | [c6834b9a…](https://stellar.expert/explorer/testnet/tx/c6834b9a36be8b3ba1a2e099ed0802e834d0b27f89943c548c54c91adf12ced9) |
| Second $30 transfer rejected on-chain (`Error(Contract, #5)`, CapExceeded) | [255681d5…](https://stellar.expert/explorer/testnet/tx/255681d59fcfc243dc51396017e5bc6be0a719fbe5e8ae6020228d5f61197e49) |
| Mandate revoked | [43b24e03…](https://stellar.expert/explorer/testnet/tx/43b24e037eadfe15bab3e2c5cc4b716df34f4a6db818948e3f9368f6f0f5781e) |
| $1 transfer after revocation rejected on-chain (`Error(Contract, #3)`, MandateRevoked) | [ecefdd52…](https://stellar.expert/explorer/testnet/tx/ecefdd529e0dddd2fa2fb103f4722691297b724d82b7d1c9ef125edc2dc03594) |

## How it works

```
owner ──create_mandate / revoke_mandate──►  Allowance account (custom account, holds funds)
                                               │
agent ──signs auth with session key──► token.transfer(account → payee, amount)
                                               │
                                   host calls account.__check_auth
                                   ├─ session key known and signature valid
                                   ├─ mandate not revoked, not expired
                                   ├─ call is token.transfer from this account
                                   ├─ destination allowlisted
                                   └─ spent this epoch + amount ≤ cap  → else reject
```

Contract: [`contracts/allowance/src/lib.rs`](contracts/allowance/src/lib.rs)

| Function | Auth | Purpose |
|---|---|---|
| `__constructor(owner, issuer)` | | Sets the owner and the Registry-stub issuer |
| `set_verified(subject, verified)` | issuer | Registry stub |
| `create_mandate(session_key, token, cap_per_epoch, epoch_secs, expires_at, allowed_destinations)` | owner, verified | Returns a mandate id |
| `revoke_mandate(mandate_id)` | owner | Immediate |
| `remaining(mandate_id)` | | Spendable now in the current epoch |
| `withdraw(token, to, amount)` | owner | Owner's own funds, not mandate-limited |
| `bump(mandate_id)` | | Keeper hook to extend storage TTL |

Events: `MandateCreated`, `MandateRevoked`, `AllowanceConsumed`, `VerifiedSet`.

Error codes: 1 NotVerified, 2 UnknownSessionKey, 3 MandateRevoked, 4 MandateExpired, 5 CapExceeded, 6 DestinationNotAllowed, 7 TokenNotAllowed, 8 FunctionNotAllowed, 9 BadArgs, 10 InvalidAmount, 11 InvalidMandate, 12 SessionKeyInUse, 13 UnknownMandate.

Amounts are integers in the token's base units (7 decimals: 50 USDC = `500000000`).

## Settlement Router

Contract: [`contracts/router/src/lib.rs`](contracts/router/src/lib.rs)

When a bank debit clears off-chain, the settlement authority calls `settle(payment_id, destination, amount)`. The router pays from its pre-funded float into the destination (the owner's mandate account), stores the payment ID so the same debit can never settle twice, and emits `Settled{payment_id, token, destination, amount}`, which ties the bank payment to the on-chain transfer.

`scripts/settle-server.js` receives the "payment cleared" webhook, checks its HMAC signature, maps it to a settlement (one function, `parseClearedDebit`, knows the payload shape) and calls the router. Redelivered webhooks return `already_settled`.

Router errors: 1 DuplicatePayment, 2 InvalidAmount, 3 InvalidPaymentId.

## Run the tests

Requires Rust with the `wasm32v1-none` target and the [Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```sh
cargo test
stellar contract build
```

Allowance tests cover: in-cap transfer passes, over-cap rejected, expired rejected, revoked rejected, cap resets next epoch, non-allowlisted destination rejected, unknown session key rejected, unverified owner cannot create a mandate. Router tests cover: settles once per payment ID (duplicate rejected), only the settlement authority can settle, invalid input rejected.

## Run the demo on testnet

```sh
cd scripts && npm install
cp ../.env.example ../.env   # fill in testnet values
set -a; . ../.env; set +a
node demo.js
```

The demo creates a $50/day mandate for a fresh agent session key, then: $30 accepted, a second $30 rejected on-chain, revoke, $1 rejected on-chain. It prints an explorer link for each transaction.

Rejected transfers are submitted on purpose. Simulation would catch them locally, but the point is a failed transaction on the public ledger, so the script reuses the footprint of the last accepted transfer and submits anyway.

## License

Apache-2.0
