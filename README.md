# OpenFi on Stellar: Allowance State Machine (prototype)

OpenFi lets treasury teams and AI agents pull funds from bank accounts under on-chain spending limits, with cleared funds settling on Stellar. This repository holds the first slice of the Stellar layer: a Soroban **custom-account** contract that enforces an agent's mandate. An agent holds a session key that can only move funds within a per-epoch cap, before an expiry, to allowlisted destinations, and only until the owner revokes it. Every check runs in `__check_auth`, so a transfer that breaks the mandate fails on-chain.

## Status

| Part | State |
|---|---|
| Allowance State Machine (cap per epoch, expiry, revoke, destination allowlist) | **Real**, deployed on testnet |
| Credential Registry check at mandate creation | **Stubbed**: an issuer-maintained allowlist (`set_verified`). The full Registry is a funded deliverable. |
| Sandbox bank debit to Stellar testnet settlement | In progress |
| One token and one session key per mandate | Prototype limits. Multi-token and multi-key mandates are funded work. |

## Testnet evidence

Run of 2026-11-05 using the testnet native-asset contract as the token. The final evidence run uses testnet USDC.

| Item | Link |
|---|---|
| Allowance account contract | [CBQTIOUH…KH5L](https://stellar.expert/explorer/testnet/contract/CBQTIOUHNF55RRED3TOHP6NYGC3B6Q72W5JYYEW2WXZNJSW4M4H7KH5L) |
| Mandate #8 created ($50/day cap) | [c7509809…](https://stellar.expert/explorer/testnet/tx/c7509809bfdfedd9d2b36bc7bc58be685107f426eaf2a90b0ad6306396780b93) |
| $30 transfer accepted | [e74537f7…](https://stellar.expert/explorer/testnet/tx/e74537f74b197a1ed7b1bd5485727849142305a0ff23d4b66f3105c074bc51ed) |
| Second $30 transfer rejected on-chain (`Error(Contract, #5)`, CapExceeded) | [53684c3d…](https://stellar.expert/explorer/testnet/tx/53684c3d18c0ce7ec063a552307654e0054c5428d63446a2338f2b13178f8450) |
| Mandate revoked | [d1c93a17…](https://stellar.expert/explorer/testnet/tx/d1c93a17f27e0c1100b120cdcefd8b92bc042dbdf6f0d60d28dadd9ca2fe5c77) |
| $1 transfer after revocation rejected on-chain (`Error(Contract, #3)`, MandateRevoked) | [096d0c57…](https://stellar.expert/explorer/testnet/tx/096d0c57e29bc5957d29de2912718f2ca66168bb839419c8bbfb2c86fc717a08) |

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

## Run the tests

Requires Rust with the `wasm32v1-none` target and the [Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```sh
cargo test
stellar contract build
```

Tests cover: in-cap transfer passes, over-cap rejected, expired rejected, revoked rejected, cap resets next epoch, non-allowlisted destination rejected, unknown session key rejected, unverified owner cannot create a mandate.

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
