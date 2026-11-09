// One-off USD bank debit in the open banking sandbox, tagged with the mandate
// account it should settle into.
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

/** Link the sandbox payer's test bank account for a debit of `usd`. */
export async function linkBank({ base, usd, scenario = 'paid' }) {
  const amountCents = Math.round(Number(usd) * 100);
  const { customerId } = await post(base, '/api/checkout/start', { amountCents, scenario });
  const paykey = await post(base, '/api/paykey', { customerId, bankId: 'fe' });
  return { base, scenario, customerId, paykey };
}

/** "First Example Bank · Checking ••••4821" */
export const maskedAccount = ({ paykey }) => {
  const type = paykey.accountType ? paykey.accountType[0].toUpperCase() + paykey.accountType.slice(1) : 'Account';
  return `${paykey.bankName} · ${type} ••••${paykey.last4}`;
};

/** Debit the linked account. The payer name must match the sandbox identity. */
export async function debitLinked(link, destination) {
  const { payment } = await post(link.base, '/api/charges', {
    customerId: link.customerId,
    paykeyId: link.paykey.id,
    fullName: 'Jordan Rivera',
    agreed: true,
    scenario: link.scenario,
    metadata: { destination },
  });
  return payment;
}

export const describeDebit = (payment, link) =>
  `debit ${payment.chargeId}: $${(payment.invoiceCents / 100).toFixed(2)} (+$${(payment.feeCents / 100).toFixed(2)} fee) from ${maskedAccount(link)}, status ${payment.status}`;

if (import.meta.url === `file://${process.argv[1]}`) {
  const [usd = '30', scenario = 'paid'] = process.argv.slice(2);
  const link = await linkBank({ base: need('BANK_API_BASE'), usd, scenario });
  const payment = await debitLinked(link, need('MANDATE_ACCOUNT'));
  console.log(describeDebit(payment, link));
  console.log(payment.chargeId);
}
