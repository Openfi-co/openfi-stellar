// One-command demo: an agent spends under a $50/day mandate, enforced on-chain.
//
//   node demo.js
//
// Env: ACCOUNT_CONTRACT (deployed allowance account), TOKEN (SAC address),
//      PAYEE (G... address), OWNER_SECRET, RELAYER_SECRET.
import { Keypair, xdr } from '@stellar/stellar-sdk';
import {
  addr, agentTransfer, bytes, contractUrl, i128, invoke, kp, read, txUrl, u64, units,
} from './lib.js';

const need = (k) => process.env[k] ?? (console.error(`missing env ${k}`), process.exit(1));
const ACCOUNT = need('ACCOUNT_CONTRACT');
const TOKEN = need('TOKEN');
const PAYEE = need('PAYEE');
const owner = kp(need('OWNER_SECRET'));
const relayer = kp(need('RELAYER_SECRET'));

const DAY = 86_400;
const rows = [];
const log = (step, r) => {
  const status = r.ok ? 'SUCCESS' : 'FAILED (rejected on-chain)';
  rows.push({ step, status, tx: txUrl(r.hash) });
  console.log(`\n${step}\n  ${status}\n  ${txUrl(r.hash)}`);
};

console.log(`Allowance account: ${contractUrl(ACCOUNT)}`);

// A fresh session key for the agent. Only its public key goes on-chain.
const agent = Keypair.random();

// 1. Owner creates a $50/day mandate for the agent.
const created = await invoke(ACCOUNT, 'create_mandate', [
  bytes(agent.rawPublicKey()),
  addr(TOKEN),
  i128(units(50)),
  u64(DAY),
  u64(Math.floor(Date.now() / 1000) + 30 * DAY),
  xdr.ScVal.scvVec([addr(PAYEE)]),
], owner);
const mandateId = created.returnValue;
log(`Mandate #${mandateId} created: $50/day cap, 30-day expiry, 1 allowed payee`, created);

const pay = (amount) =>
  agentTransfer({ token: TOKEN, account: ACCOUNT, to: PAYEE, amount: units(amount), session: agent, relayer });

// 2. $30: within cap.
log('$30 transfer signed by the agent session key', await pay(30));
console.log(`  remaining today: $${Number(await read(ACCOUNT, 'remaining', [u64(mandateId)], relayer)) / 1e7}`);

// 3. Another $30: exceeds the $50/day cap.
log('Second $30 transfer (would total $60 > $50 cap)', await pay(30));

// 4. Owner revokes the mandate.
log(`Mandate #${mandateId} revoked by owner`, await invoke(ACCOUNT, 'revoke_mandate', [u64(mandateId)], owner));

// 5. $1 after revocation.
log('$1 transfer after revocation', await pay(1));

console.log('\nSummary');
console.table(rows);
