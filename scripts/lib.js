// Shared helpers for the OpenFi Stellar demo scripts.
import {
  Address,
  BASE_FEE,
  Contract,
  Keypair,
  Networks,
  Operation,
  SorobanDataBuilder,
  TransactionBuilder,
  hash,
  nativeToScVal,
  rpc,
  scValToNative,
  xdr,
} from '@stellar/stellar-sdk';

export const RPC_URL = process.env.STELLAR_RPC_URL ?? 'https://soroban-testnet.stellar.org';
export const PASSPHRASE = process.env.STELLAR_NETWORK_PASSPHRASE ?? Networks.TESTNET;
export const EXPLORER = process.env.STELLAR_EXPLORER ?? 'https://stellar.expert/explorer/testnet';
const t0 = Date.now();
export const dbg = (m) => process.env.DEBUG && console.error(`  [${((Date.now() - t0) / 1000).toFixed(1)}s] ${m}`);

// Testnet RPC occasionally leaves a request hanging. Time out and retry;
// resending the same signed transaction is safe (it comes back DUPLICATE).
const rawServer = new rpc.Server(RPC_URL, { timeout: 15_000 });
export const server = new Proxy(rawServer, {
  get(target, prop) {
    const v = target[prop];
    if (typeof v !== 'function') return v;
    return async (...args) => {
      for (let attempt = 1; ; attempt++) {
        try {
          return await Promise.race([
            v.apply(target, args),
            new Promise((_, rej) => setTimeout(() => rej(new Error('timeout (client)')), 20_000)),
          ]);
        } catch (e) {
          const transient = /timeout|ECONNRESET|socket hang up|ETIMEDOUT|503|502|Account not found/i.test(String(e?.message ?? e));
          if (!transient || attempt >= 4) throw e;
          dbg(`rpc ${String(prop)} retry ${attempt}: ${e.message ?? e}`);
        }
      }
    };
  },
});

export const txUrl = (h) => `${EXPLORER}/tx/${h}`;
export const contractUrl = (id) => `${EXPLORER}/contract/${id}`;

/** 7-decimal token units. usdc(50) === 500_000_000n */
export const units = (n) => BigInt(Math.round(n * 1e7));

export const addr = (a) => new Address(a).toScVal();
export const i128 = (n) => nativeToScVal(BigInt(n), { type: 'i128' });
export const u64 = (n) => nativeToScVal(BigInt(n), { type: 'u64' });
export const bytes = (b) => xdr.ScVal.scvBytes(Buffer.from(b));

async function waitFor(hashHex) {
  dbg(`waiting for ${hashHex.slice(0, 8)}`);
  for (let i = 0; i < 60; i++) {
    const r = await server.getTransaction(hashHex);
    if (r.status !== rpc.Api.GetTransactionStatus.NOT_FOUND) return r;
    await new Promise((res) => setTimeout(res, 1000));
  }
  throw new Error(`timed out waiting for ${hashHex}`);
}

async function send(tx, signer) {
  tx.sign(signer);
  let sent = await server.sendTransaction(tx);
  for (let i = 0; sent.status === 'TRY_AGAIN_LATER' && i < 10; i++) {
    await new Promise((res) => setTimeout(res, 2000));
    sent = await server.sendTransaction(tx);
  }
  dbg(`send ${sent.status} ${sent.hash}`);
  if (sent.status === 'DUPLICATE') dbg('duplicate send (already in flight)');
  if (sent.status === 'ERROR') {
    throw new Error(`send failed: ${sent.errorResult?.result().switch().name ?? 'unknown'}`);
  }
  const res = await waitFor(sent.hash);
  return {
    hash: sent.hash,
    ok: res.status === rpc.Api.GetTransactionStatus.SUCCESS,
    returnValue: res.returnValue ? scValToNative(res.returnValue) : undefined,
  };
}

/** Rebuild from scratch if a transaction expired before it was sent. */
async function rebuildIfLate(fn) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await fn();
    } catch (e) {
      if (!/txTooLate/.test(String(e?.message)) || attempt >= 3) throw e;
      dbg('transaction expired before send; rebuilding');
    }
  }
}

/** Standard invoke: simulate, assemble, sign with `source`, submit. */
export const invoke = (...a) => rebuildIfLate(() => invokeOnce(...a));
async function invokeOnce(contractId, method, args, source) {
  const account = await server.getAccount(source.publicKey());
  const tx = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: PASSPHRASE })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(60)
    .build();
  dbg(`${method}: preparing`);
  const prepared = await server.prepareTransaction(tx);
  dbg(`${method}: submitting`);
  const r = await send(prepared, source);
  if (!r.ok) throw new Error(`${method} failed on-chain: ${txUrl(r.hash)}`);
  return r;
}

/** Read-only call via simulation. */
export async function read(contractId, method, args, source) {
  const account = await server.getAccount(source.publicKey());
  const tx = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: PASSPHRASE })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(60)
    .build();
  const sim = await server.simulateTransaction(tx);
  if (rpc.Api.isSimulationError(sim)) throw new Error(sim.error);
  return scValToNative(sim.result.retval);
}

/** Sign a Soroban auth entry for the allowance account with the agent's session key. */
function signWithSessionKey(entry, session, validUntil) {
  const creds = entry.credentials().address();
  creds.signatureExpirationLedger(validUntil);
  const preimage = xdr.HashIdPreimage.envelopeTypeSorobanAuthorization(
    new xdr.HashIdPreimageSorobanAuthorization({
      networkId: hash(Buffer.from(PASSPHRASE)),
      nonce: creds.nonce(),
      signatureExpirationLedger: validUntil,
      invocation: entry.rootInvocation(),
    }),
  );
  const sig = session.sign(hash(preimage.toXDR()));
  // Matches the contract's `SessionSignature` struct (map keys sorted).
  creds.signature(
    xdr.ScVal.scvMap([
      new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol('public_key'), val: bytes(session.rawPublicKey()) }),
      new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol('signature'), val: bytes(sig) }),
    ]),
  );
  return entry;
}

function keysOf(data) {
  const fp = data.resources().footprint();
  return { ro: fp.readOnly(), rw: fp.readWrite() };
}

function union(a, b) {
  const seen = new Map();
  for (const k of [...a, ...b]) seen.set(k.toXDR('base64'), k);
  return [...seen.values()];
}

// Footprint and resources from the last transfer that passed `__check_auth`.
// A transfer the contract will reject cannot be simulated in enforcing mode,
// so to put the rejection on-chain we reuse this footprint (same mandate and
// token entries; only the auth nonce differs).
let lastGood = null;

/**
 * The agent pays `to` from the allowance account: `token.transfer(account, to, amount)`,
 * authorized only by the session key. `relayer` pays the network fee.
 *
 * When the contract would reject the transfer we still submit it, so the
 * rejection is a failed transaction on the public ledger, not a local error.
 */
export const agentTransfer = (o) => rebuildIfLate(() => agentTransferOnce(o));
async function agentTransferOnce({ token, account, to, amount, session, relayer }) {
  const call = new Contract(token).call('transfer', addr(account), addr(to), i128(amount));
  const build = async (op) =>
    new TransactionBuilder(await server.getAccount(relayer.publicKey()), {
      fee: BASE_FEE,
      networkPassphrase: PASSPHRASE,
    })
      .addOperation(op)
      .setTimeout(60)
      .build();

  dbg('transfer: recording simulation');
  // 1. Recording-mode simulation: yields the auth entry (with a fresh nonce).
  const recSim = await server.simulateTransaction(await build(call));
  if (rpc.Api.isSimulationError(recSim)) throw new Error(recSim.error);
  const { sequence } = await server.getLatestLedger();
  const entries = recSim.result.auth.map((e) => signWithSessionKey(e, session, sequence + 100));

  const func = call.body().invokeHostFunctionOp().hostFunction();
  const signedOp = Operation.invokeHostFunction({ func, auth: entries });

  // 2. Enforcing simulation: runs `__check_auth`, so the footprint includes
  //    the mandate's storage.
  const tx = await build(signedOp);
  dbg('transfer: enforcing simulation');
  const sim = await server.simulateTransaction(tx);

  if (!rpc.Api.isSimulationError(sim)) {
    lastGood = sim.transactionData.build();
    const r = await send(rpc.assembleTransaction(tx, sim).build(), relayer);
    return { ...r, expected: 'accepted' };
  }

  // 3. Contract will reject. Submit anyway with a known-good footprint.
  if (!lastGood) throw new Error(`rejected in simulation and no prior footprint: ${sim.error}`);
  const rec = keysOf(recSim.transactionData.build());
  const good = keysOf(lastGood);
  const res = lastGood.resources();
  const data = new SorobanDataBuilder()
    .setReadOnly(union(good.ro, rec.ro))
    .setReadWrite(union(good.rw, rec.rw))
    .setResources(res.instructions() * 2, res.diskReadBytes() * 2, res.writeBytes() * 2)
    .setResourceFee(BigInt(lastGood.resourceFee().toString()) * 3n)
    .build();
  const forced = TransactionBuilder.cloneFrom(tx, {
    fee: (BigInt(BASE_FEE) + BigInt(data.resourceFee().toString())).toString(),
    sorobanData: data,
  }).build();
  const r = await send(forced, relayer);
  return { ...r, expected: 'rejected', simError: sim.error };
}

export const kp = (secret) => Keypair.fromSecret(secret);

const ALLOWANCE_ERRORS = {
  1: 'NotVerified', 2: 'UnknownSessionKey', 3: 'MandateRevoked', 4: 'MandateExpired', 5: 'CapExceeded',
  6: 'DestinationNotAllowed', 7: 'TokenNotAllowed', 8: 'FunctionNotAllowed', 9: 'BadArgs',
  10: 'InvalidAmount', 11: 'InvalidMandate', 12: 'SessionKeyInUse', 13: 'UnknownMandate',
};

/** The allowance contract error recorded in a failed transaction, e.g. "Error(Contract, #5) CapExceeded". */
export async function contractError(hashHex) {
  const r = await server.getTransaction(hashHex);
  for (const e of r.diagnosticEventsXdr ?? []) {
    for (const t of e.event().body().v0().topics()) {
      if (t.switch().name === 'scvError' && t.error().switch().name === 'sceContract') {
        const code = t.error().contractCode();
        return `Error(Contract, #${code}) ${ALLOWANCE_ERRORS[code] ?? ''}`.trim();
      }
    }
  }
  return null;
}
