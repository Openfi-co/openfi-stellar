// Starts a one-off USD bank debit in the open banking sandbox, tagged with
// the mandate account it should settle into. Prints the charge ID.
//
//   node start-debit.js <amount-usd> [scenario]     scenario: paid | returned | failed_insufficient_funds
//
// Env: BANK_API_BASE, MANDATE_ACCOUNT.
const need = (k) => process.env[k] ?? (console.error(`missing env ${k}`), process.exit(1));
const BASE = need('BANK_API_BASE');
const MANDATE_ACCOUNT = need('MANDATE_ACCOUNT');
const [usd = '30', scenario = 'paid'] = process.argv.slice(2);
const amountCents = Math.round(Number(usd) * 100);

async function post(path, body) {
  const res = await fetch(`${BASE}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const json = await res.json();
  if (!res.ok) throw new Error(`${path} ${res.status}: ${JSON.stringify(json)}`);
  return json;
}

// Sandbox payer and test account; the payer name must match the sandbox identity.
const { customerId } = await post('/api/checkout/start', { amountCents, scenario });
const paykey = await post('/api/paykey', { customerId, bankId: 'fe' });
const { payment } = await post('/api/charges', {
  customerId,
  paykeyId: paykey.id,
  fullName: 'Jordan Rivera',
  agreed: true,
  scenario,
  metadata: { destination: MANDATE_ACCOUNT },
});
console.log(`debit ${payment.chargeId}: $${(payment.invoiceCents / 100).toFixed(2)} (+$${(payment.feeCents / 100).toFixed(2)} fee) from ${paykey.label}, status ${payment.status}`);
console.log(payment.chargeId);
