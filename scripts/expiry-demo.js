// Expiry on-chain: a mandate that expires 60 seconds after creation accepts
// a transfer before expiry and rejects the same transfer after it.
//
//   node expiry-demo.js        (same env as demo.js)
import { Keypair, xdr } from '@stellar/stellar-sdk';
import { addr, agentTransfer, bytes, contractUrl, i128, invoke, kp, txUrl, u64, units } from './lib.js';

const need = (k) => process.env[k] ?? (console.error(`missing env ${k}`), process.exit(1));
const ACCOUNT = need('ACCOUNT_CONTRACT');
const TOKEN = need('TOKEN');
const PAYEE = need('PAYEE');
const owner = kp(need('OWNER_SECRET'));
const relayer = kp(need('RELAYER_SECRET'));

const EXPIRES_IN = 60;
const rows = [];
const log = (step, r) => {
  const status = r.ok ? 'SUCCESS' : 'FAILED (rejected on-chain)';
  rows.push({ step, status, tx: txUrl(r.hash) });
  console.log(`\n${step}\n  ${status}\n  ${txUrl(r.hash)}`);
};

console.log(`Allowance account: ${contractUrl(ACCOUNT)}`);
const agent = Keypair.random();
const expiresAt = Math.floor(Date.now() / 1000) + EXPIRES_IN;

const created = await invoke(ACCOUNT, 'create_mandate', [
  bytes(agent.rawPublicKey()),
  addr(TOKEN),
  i128(units(50)),
  u64(86_400),
  u64(expiresAt),
  xdr.ScVal.scvVec([addr(PAYEE)]),
], owner);
log(`Mandate #${created.returnValue} created: $50/day cap, expires in ${EXPIRES_IN}s`, created);

const pay = () =>
  agentTransfer({ token: TOKEN, account: ACCOUNT, to: PAYEE, amount: units(1), session: agent, relayer });

log('$1 transfer before expiry', await pay());

const wait = expiresAt + 15 - Math.floor(Date.now() / 1000);
console.log(`\nwaiting ${wait}s for the mandate to expire...`);
await new Promise((r) => setTimeout(r, Math.max(0, wait) * 1000));

log('$1 transfer after expiry', await pay());

console.log('\nSummary');
console.table(rows);
