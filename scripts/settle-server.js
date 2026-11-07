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
//      (default destination), WEBHOOK_SECRET, PORT (default 8787).
import { createHmac, timingSafeEqual } from 'node:crypto';
import { createServer } from 'node:http';
import { addr, i128, invoke, kp, read, txUrl, units } from './lib.js';
import { nativeToScVal } from '@stellar/stellar-sdk';

const need = (k) => process.env[k] ?? (console.error(`missing env ${k}`), process.exit(1));
const ROUTER = need('ROUTER_CONTRACT');
const authority = kp(need('SETTLEMENT_SECRET'));
const MANDATE_ACCOUNT = need('MANDATE_ACCOUNT');
const WEBHOOK_SECRET = need('WEBHOOK_SECRET');
const PORT = Number(process.env.PORT ?? 8787);

const str = (s) => nativeToScVal(s, { type: 'string' });

/**
 * Map the provider's webhook body to a settlement instruction, or null if the
 * event is not a cleared debit. This is the only place that knows the
 * payload shape. PLACEHOLDER shape until confirmed against the sandbox:
 *   { "event": "payment.cleared",
 *     "data": { "id": "...", "amount": "30.00", "currency": "USD",
 *               "metadata": { "destination": "C..." } } }
 */
function parseClearedDebit(body) {
  if (body?.event !== 'payment.cleared') return null;
  const d = body.data ?? {};
  if (d.currency !== 'USD') throw new Error(`unsupported currency ${d.currency}`);
  const amount = Number(d.amount);
  if (!d.id || !(amount > 0)) throw new Error('missing payment id or amount');
  return { paymentId: String(d.id), amount, destination: d.metadata?.destination ?? MANDATE_ACCOUNT };
}

/** HMAC-SHA256 of the raw body, hex, in `x-webhook-signature`. Confirm header and scheme against the sandbox. */
function verified(raw, header) {
  if (!header) return false;
  const expected = createHmac('sha256', WEBHOOK_SECRET).update(raw).digest('hex');
  const a = Buffer.from(expected);
  const b = Buffer.from(String(header));
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
    if (!verified(raw, req.headers['x-webhook-signature'])) return json(res, 401, { error: 'bad_signature' });
    try {
      const instruction = parseClearedDebit(JSON.parse(raw.toString('utf8')));
      if (!instruction) return json(res, 200, { status: 'ignored' });
      console.log(`cleared debit ${instruction.paymentId}: $${instruction.amount} -> ${instruction.destination}`);
      const out = await settle(instruction);
      console.log(`  ${out.status}${out.tx ? ` ${out.tx}` : ''}`);
      json(res, 200, { payment_id: instruction.paymentId, ...out });
    } catch (e) {
      console.error(`  error: ${e.message}`);
      json(res, 500, { error: 'settlement_failed' });
    }
  });
}).listen(PORT, () => console.log(`listening on :${PORT}/webhooks/bank-debit`));
