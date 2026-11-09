// Starts a one-off USD bank debit in the open banking sandbox, tagged with
// the mandate account it should settle into. Prints the charge ID.
//
//   node start-debit.js <amount-usd> [scenario]     scenario: paid | returned | failed_insufficient_funds
//
// Env: BANK_API_BASE, MANDATE_ACCOUNT.
const need = (k) => process.env[k] ?? (console.error(`missing env ${k}`), process.exit(1));

async function post(base, path, body) {
  const res = await fetch(`${base}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const json = await res.json();
  if (!res.ok) throw new Error(`${path} ${res.status}: ${JSON.stringify(json)}`);
  return json;
}

/** Start a one-off sandbox debit tagged with `destination`. Returns the charge and the account label. */
export async function startDebit({ base, destination, usd, scenario = 'paid' }) {
  const amountCents = Math.round(Number(usd) * 100);
  // Sandbox payer and test account; the payer name must match the sandbox identity.
  const { customerId } = await post(base, '/api/checkout/start', { amountCents, scenario });
  const paykey = await post(base, '/api/paykey', { customerId, bankId: 'fe' });
  const { payment } = await post(base, '/api/charges', {
    customerId,
    paykeyId: paykey.id,
    fullName: 'Jordan Rivera',
    agreed: true,
    scenario,
    metadata: { destination },
  });
  return { payment, account: paykey.label };
}

export const describeDebit = ({ payment, account }) =>
  `debit ${payment.chargeId}: $${(payment.invoiceCents / 100).toFixed(2)} (+$${(payment.feeCents / 100).toFixed(2)} fee) from ${account}, status ${payment.status}`;

if (import.meta.url === `file://${process.argv[1]}`) {
  const [usd = '30', scenario = 'paid'] = process.argv.slice(2);
  const debit = await startDebit({ base: need('BANK_API_BASE'), destination: need('MANDATE_ACCOUNT'), usd, scenario });
  console.log(describeDebit(debit));
  console.log(debit.payment.chargeId);
}
