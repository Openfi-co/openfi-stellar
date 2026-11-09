// Local test only: posts a signed status webhook to settle-server.js in the
// sandbox's format. Real runs receive the webhook from the sandbox itself.
//
//   node send-test-webhook.js <charge-id> [status]
import { createHmac } from 'node:crypto';

const [chargeId, status = 'paid'] = process.argv.slice(2);
if (!chargeId) (console.error('usage: node send-test-webhook.js <charge-id> [status]'), process.exit(1));
const url = process.env.WEBHOOK_URL ?? `http://localhost:${process.env.PORT ?? 8787}/webhooks/bank-debit`;
const body = JSON.stringify({ data: { id: chargeId, status } });
const ts = String(Math.floor(Date.now() / 1000));
const sig = createHmac('sha256', process.env.WEBHOOK_SECRET ?? '').update(`${ts}.${body}`).digest('hex');

const res = await fetch(url, {
  method: 'POST',
  headers: {
    'content-type': 'application/json',
    [process.env.WEBHOOK_SIGNATURE_HEADER ?? 'x-webhook-signature']: sig,
    [process.env.WEBHOOK_TIMESTAMP_HEADER ?? 'x-webhook-timestamp']: ts,
  },
  body,
});
console.log(res.status, await res.text());
