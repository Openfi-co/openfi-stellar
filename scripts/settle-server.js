// Bank-debit settlement listener.
//
// Receives the "payment cleared" webhook from Flick's open banking rails and
// settles the same amount in USDC on Stellar through the Settlement Router,
// into the owner's mandate account. The bank payment ID is recorded on-chain
// in the router's `Settled` event and can never settle twice.
//
//   node settle-server.js
//
// Env: ROUTER_CONTRACT, SETTLEMENT_SECRET (router authority), MANDATE_ACCOUNT
//      (default destination), WEBHOOK_SECRET, BANK_API_BASE (sandbox base URL),
//      WEBHOOK_SIGNATURE_HEADER, WEBHOOK_TIMESTAMP_HEADER, PORT (default 8787).
import { createHmac, timingSafeEqual } from 'node:crypto';
import { createServer } from 'node:http';
import { addr, i128, invoke, kp, read, txUrl, units } from './lib.js';
import { nativeToScVal } from '@stellar/stellar-sdk';

const need = (k) => process.env[k] ?? (console.error(`missing env ${k}`), process.exit(1));
const ROUTER = need('ROUTER_CONTRACT');
const authority = kp(need('SETTLEMENT_SECRET'));
const MANDATE_ACCOUNT = need('MANDATE_ACCOUNT');
const WEBHOOK_SECRET = need('WEBHOOK_SECRET');
const BANK_API_BASE = need('BANK_API_BASE');
const SIG_HEADER = (process.env.WEBHOOK_SIGNATURE_HEADER ?? 'x-webhook-signature').toLowerCase();
const TS_HEADER = (process.env.WEBHOOK_TIMESTAMP_HEADER ?? 'x-webhook-timestamp').toLowerCase();
const MAX_SKEW_SECS = 300;
const PORT = Number(process.env.PORT ?? 8787);

const str = (s) => nativeToScVal(s, { type: 'string' });

/**
 * The webhook carries only the charge ID and status: {"data":{"id":"chg_...","status":"paid"}}.
 * `paid` means the debit cleared. Amount and destination come from the charge
 * itself, so a forged or stale body cannot change what settles.
 */
async function parseClearedDebit(body) {
  const { id, status, return_code: returnCode } = body?.data ?? {};
  if (!id) throw new Error('missing charge id');
  if (status === 'returned') {
    // Returns arrive after `paid` (days later in production). Reversal on
    // Stellar is funded work; record it so it is visible.
    console.warn(`  RETURNED ${id} (${returnCode ?? 'no code'}): funds already settled must be recovered`);
    return null;
  }
  if (status !== 'paid') return null;
  const res = await fetch(`${BANK_API_BASE}/api/charges/${encodeURIComponent(id)}`);
  if (!res.ok) throw new Error(`charge lookup ${res.status}`);
  const { payment } = await res.json();
  // The signed webhook is the clearing signal; the lookup supplies amount and
  // destination and blocks charges the bank side already marked bad.
  if (['failed', 'cancelled', 'returned'].includes(payment?.status)) throw new Error(`charge ${id} is ${payment.status}`);
  // Settle the invoice amount; the 1% rail fee is not converted.
  const cents = payment.invoiceCents ?? payment.amountCents;
  if (!(cents > 0)) throw new Error('missing amount');
  return { paymentId: id, amount: cents / 100, destination: payment.metadata?.destination ?? MANDATE_ACCOUNT };
}

/** Hex HMAC-SHA256 over `${timestamp}.${rawBody}`; reject stale timestamps. */
function verified(raw, sig, ts) {
  if (!sig || !ts || Math.abs(Date.now() / 1000 - Number(ts)) > MAX_SKEW_SECS) return false;
  const expected = createHmac('sha256', WEBHOOK_SECRET).update(`${ts}.`).update(raw).digest('hex');
  const a = Buffer.from(expected);
  const b = Buffer.from(String(sig));
  return a.length === b.length && timingSafeEqual(a, b);
}

async function settle({ paymentId, amount, destination }) {
  const already = await read(ROUTER, 'settled_at', [str(paymentId)], authority);
  if (already != null) return { status: 'already_settled', ledger: already };
  const r = await invoke(ROUTER, 'settle', [str(paymentId), addr(destination), i128(units(amount))], authority);
  return { status: 'settled', tx: txUrl(r.hash) };
}

const json = (res, code, obj) => {
  res.writeHead(code, { 'content-type': 'application/json' });
  res.end(JSON.stringify(obj));
};

createServer((req, res) => {
  if (req.method !== 'POST' || req.url !== '/webhooks/bank-debit') return json(res, 404, { error: 'not_found' });
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', async () => {
    const raw = Buffer.concat(chunks);
    if (!verified(raw, req.headers[SIG_HEADER], req.headers[TS_HEADER])) return json(res, 401, { error: 'bad_signature' });
    try {
      const instruction = await parseClearedDebit(JSON.parse(raw.toString('utf8')));
      if (!instruction) return json(res, 200, { status: 'ignored' });
      // Acknowledge at once; on-chain settlement takes a few seconds and the
      // sender should not time out and redeliver while it runs.
      json(res, 202, { payment_id: instruction.paymentId, status: 'accepted' });
      console.log(`cleared debit ${instruction.paymentId}: $${instruction.amount} -> ${instruction.destination}`);
      settle(instruction)
        .then((out) => console.log(`  ${out.status}${out.tx ? ` ${out.tx}` : ''}`))
        .catch((e) => console.error(`  settlement error: ${e.message}`));
    } catch (e) {
      console.error(`  error: ${e.message}`);
      json(res, 500, { error: 'settlement_failed' });
    }
  });
}).listen(PORT, () => console.log(`listening on :${PORT}/webhooks/bank-debit`));
