// One-command demo: an agent spends under a $50/day mandate, enforced on-chain.
//
//   node demo.js                 agent spending only
//   BANK_DEBIT=1 node demo.js    plus a sandbox bank debit that funds the account
//
// Env: ACCOUNT_CONTRACT (deployed allowance account), TOKEN (SAC address),
//      PAYEE (G... address), OWNER_SECRET, RELAYER_SECRET; with BANK_DEBIT also
//      BANK_API_BASE and ROUTER_CONTRACT, and settle-server.js running.
//      PAUSE=1 waits for Enter after the mandate is created.
import { createInterface } from 'node:readline/promises';
import { Keypair, nativeToScVal, xdr } from '@stellar/stellar-sdk';
import {
  addr, agentTransfer, bytes, contractUrl, i128, invoke, kp, read, txUrl, u64, units,
} from './lib.js';
import { debitLinked, describeDebit, linkBank, maskedAccount } from './start-debit.js';

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
console.log(`Token:             ${contractUrl(TOKEN)}${process.env.TOKEN_LABEL ? ` (${process.env.TOKEN_LABEL})` : ''}`);

// A fresh session key for the agent. Only its public key goes on-chain.
const agent = Keypair.random();

// BANK_DEBIT=1: link the owner's (sandbox) bank account first; it funds the
// mandate account through a bank debit below.
const link = process.env.BANK_DEBIT ? await linkBank({ base: need('BANK_API_BASE'), usd: 30 }) : null;
if (link) console.log(`\nBank account linked (sandbox)\n  ${maskedAccount(link)}`);

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
log(`Mandate #${mandateId} created: $50/day cap, 30-day expiry${link ? `\n  funding source: ${maskedAccount(link)}` : ''}`, created);

// The bank debit funds the mandate account; settle-server.js settles it
// through the router when the sandbox reports it paid.
if (link) {
  const payment = await debitLinked(link, ACCOUNT);
  console.log(`\nSandbox bank debit\n  ${describeDebit(payment, link)}`);
  const router = need('ROUTER_CONTRACT');
  const id = nativeToScVal(payment.chargeId, { type: 'string' });
  let ledger = null;
  for (let i = 0; i < 45 && ledger == null; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    ledger = await read(router, 'settled_at', [id], relayer);
  }
  console.log(ledger == null ? '  not settled yet: check the listener' : `  30 USDC settled on Stellar testnet in ledger ${ledger}`);
}

// PAUSE=1 stops here so another step (e.g. a bank debit) can run in between.
if (process.env.PAUSE) {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  await rl.question('\nPress Enter to continue...');
  rl.close();
}

const pay = (amount) =>
  agentTransfer({ token: TOKEN, account: ACCOUNT, to: PAYEE, amount: units(amount), session: agent, relayer });

// 2. $30: within cap.
log('$30 transfer signed by the agent session key', await pay(30));
console.log(`  remaining today: $${Number(await read(ACCOUNT, 'remaining', [u64(mandateId)], relayer)) / 1e7}`);

// 3. Another $30: exceeds the $50/day cap.
log('Second $30 transfer (would total $60 > $50 cap)', await pay(30));
console.log(`  remaining today: $${Number(await read(ACCOUNT, 'remaining', [u64(mandateId)], relayer)) / 1e7} (unchanged)`);

// 4. Owner revokes the mandate.
log(`Mandate #${mandateId} revoked by owner`, await invoke(ACCOUNT, 'revoke_mandate', [u64(mandateId)], owner));

// 5. $1 after revocation.
log('$1 transfer after revocation', await pay(1));

console.log('\nSummary');
console.table(rows);
