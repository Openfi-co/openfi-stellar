# OpenFi on Stellar: Allowance State Machine and Settlement Router (prototype)

OpenFi lets treasury teams and AI agents pull funds from bank accounts under on-chain spending limits, with cleared funds settling on Stellar. This repository holds the first slice of the Stellar layer: a Soroban **custom-account** contract that enforces an agent's mandate. An agent holds a session key that can only move funds within a per-epoch cap, before an expiry, to allowlisted destinations, and only until the owner revokes it. Every check runs in `__check_auth`, so a transfer that breaks the mandate fails on-chain.

## Status

| Part | State |
|---|---|
| Allowance State Machine (cap per epoch, expiry, revoke, destination allowlist) | **Real**, deployed on testnet |
| Credential Registry check at mandate creation | **Stubbed**: an issuer-maintained allowlist (`set_verified`). The full Registry is a funded deliverable. |
| Settlement Router: releases USDC for a cleared bank debit, idempotent per payment ID | **Real**, deployed on testnet ([CDG3QJXN…KJCI](https://stellar.expert/explorer/testnet/contract/CDG3QJXN5CSDH7MXH5FXQVLG4O5GAYXJWCBHCTK3AMCN45QA3MWLKJCI)) |
| Bank-debit webhook listener (`scripts/settle-server.js`) | **Real**. Driven end to end by a sandbox bank debit from Flick's open banking sandbox (see below). |
| Router authority | Prototype uses a single key; production uses a threshold multisig. No mandate or Registry check inside the router yet (funded work). |
| One token and one session key per mandate | Prototype limits. Multi-token and multi-key mandates are funded work. |

## Testnet evidence

Run of 2026-11-09 (UTC) with Circle's testnet USDC, the same run as the demo video. Every row is verifiable on the explorer; the two failed transactions show the contract's error code in their diagnostic events.

| Item | Link |
|---|---|
| Allowance account contract | [CCA3E57A…HIQAU](https://stellar.expert/explorer/testnet/contract/CCA3E57AE4TU3LDTKCREKXTGGJZVDOT2DXCXXSHMUGOL3EBD467HIQAU) |
| Token: USDC (testnet) | [CBIELTK6…DAMA](https://stellar.expert/explorer/testnet/contract/CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA) |
| Mandate created ($50/day cap) | [035f94e9…](https://stellar.expert/explorer/testnet/tx/035f94e94b6619b3537ffb61b262a75c3d0e346c965b32180681bc0da22f297c) |
| $30 transfer accepted | [c03df68d…](https://stellar.expert/explorer/testnet/tx/c03df68d76ab7192d4ec595cfedab99c78fb5b3028a3357f66673c8b04f166f7) |
| Second $30 transfer rejected on-chain (`Error(Contract, #5)`, CapExceeded) | [6c3e2248…](https://stellar.expert/explorer/testnet/tx/6c3e2248543299611a4c2edb3a245e06ab9a25939f4cb28c7c4611710d38d995) |
| Mandate revoked | [6874f061…](https://stellar.expert/explorer/testnet/tx/6874f06130a4629b8de6cc543943112c9f22257a7eb28086d2ce357b51215fee) |
| $1 transfer after revocation rejected on-chain (`Error(Contract, #3)`, MandateRevoked) | [ff68b656…](https://stellar.expert/explorer/testnet/tx/ff68b6564b61254b2a25aad90187f207580d9e729a80533063fb47d65ddf08c2) |
| Expiry (`scripts/expiry-demo.js`): mandate #2 created with a 60-second expiry | [4d55c212…](https://stellar.expert/explorer/testnet/tx/4d55c212d2c5bab9d45ddba35cf87a6ad5e520e997e164cff07b32e9c538a74d) |
| $1 transfer before expiry accepted | [bda72768…](https://stellar.expert/explorer/testnet/tx/bda72768d042da8fb42a9c54d1785039085613a2b50f30f8f765945091f8ac95) |
| $1 transfer after expiry rejected on-chain (`Error(Contract, #4)`, MandateExpired) | [ec465f6c…](https://stellar.expert/explorer/testnet/tx/ec465f6cd7334a98fad23f4c3758b9ffa6469dba426cc2011f04299e63ca3a82) |

### Sandbox bank debit to Stellar settlement

Same run. The owner's sandbox bank account (First Example Bank, checking ••••4821) was linked, and a $30.00 one-off USD bank debit was started in Flick's open banking sandbox (`scripts/start-debit.js`). When it cleared, the sandbox sent a signed `paid` webhook; the listener verified it, read the amount and destination from the charge, and the Settlement Router released 30 USDC into the mandate account. The bank charge ID is the topic of the on-chain `settled` event.

The sandbox's bank leg ran on its deterministic mock (`live: false`); the HTTP contract, webhook payloads and signing are the same as with the live rail.

| Item | Link |
|---|---|
| Bank charge `chg_mock_00762ff4-69e3-48cd-bcbd-32cdcf04cdd8` ($30.00 + $0.30 fee) cleared, webhook delivered | sandbox |
| Router `settle`: 30 USDC to the mandate account, `settled` event keyed by the charge ID | [e268df6e…](https://stellar.expert/explorer/testnet/tx/e268df6e4c5ad6c9deab64e5e5bdc4f74c6019e015cde37819777a024e678aaf) |

**Testnet resets.** Stellar testnet is periodically reset (next scheduled: 16 Dec 2026), which removes these contracts and transactions from the explorer. The Horizon records of every transaction above are archived in [`evidence/`](evidence/) (each includes the signed envelope, result and ledger). After a reset, redeploy and re-run `scripts/demo.js` to regenerate live links.

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
