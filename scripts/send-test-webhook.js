// Local test only: posts a signed "payment cleared" body to settle-server.js,
// in the placeholder shape. Real runs receive the webhook from the sandbox.
//
//   node send-test-webhook.js <payment-id> <amount-usd>
import { createHmac } from 'node:crypto';

const [paymentId = `local_test_${Date.now()}`, amount = '30.00'] = process.argv.slice(2);
const url = process.env.WEBHOOK_URL ?? `http://localhost:${process.env.PORT ?? 8787}/webhooks/bank-debit`;
const body = JSON.stringify({ event: 'payment.cleared', data: { id: paymentId, amount, currency: 'USD' } });
const sig = createHmac('sha256', process.env.WEBHOOK_SECRET ?? '').update(body).digest('hex');

const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json', 'x-webhook-signature': sig }, body });
console.log(res.status, await res.text());
