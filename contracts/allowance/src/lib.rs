//! OpenFi Allowance State Machine (prototype).
//!
//! A Soroban custom-account contract. The contract address holds the funds;
//! the owner creates mandates, and each mandate binds one agent session key
//! (ed25519) to a token, a per-epoch cap, an expiry and a destination
//! allowlist. `__check_auth` is the enforcement point: a token `transfer`
//! out of this account is authorized only if the session key signed it and
//! the transfer fits the mandate. Anything else is rejected on-chain.
//!
//! Prototype scope (unfunded): one token and one session key per mandate.
//! The Credential Registry check is STUBBED as an issuer-maintained
//! allowlist (`set_verified`); the real Registry is a funded deliverable.
//!
//! Amounts are i128 in the token's base units (7 decimals for Stellar
//! assets: 50 USDC = 500_000_000). No floats.
#![no_std]

use soroban_sdk::{
    auth::{Context, ContractContext, CustomAccountInterface},
    contract, contracterror, contractevent, contractimpl, contracttype,
    crypto::Hash,
    symbol_short, token, Address, BytesN, Env, TryFromVal, Val, Vec,
};

// ------------------------------------------------------------------ errors

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotVerified = 1,
    UnknownSessionKey = 2,
    MandateRevoked = 3,
    MandateExpired = 4,
    CapExceeded = 5,
    DestinationNotAllowed = 6,
    TokenNotAllowed = 7,
    FunctionNotAllowed = 8,
    BadArgs = 9,
    InvalidAmount = 10,
    InvalidMandate = 11,
    SessionKeyInUse = 12,
    UnknownMandate = 13,
}

// ----------------------------------------------------------------- storage

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mandate {
    pub session_key: BytesN<32>,
    pub token: Address,
    pub cap_per_epoch: i128,
    pub epoch_secs: u64,
    /// Start of the epoch being consumed. Advances in whole epochs so the
    /// boundary keeps the original cadence.
    pub epoch_start: u64,
    pub spent_this_epoch: i128,
    pub expires_at: u64,
    pub allowed_destinations: Vec<Address>,
    pub revoked: bool,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Owner,
    Issuer,
    NextId,
    /// Registry stub: subject -> verified.
    Verified(Address),
    Mandate(u64),
    /// Session key -> mandate id.
    KeyIndex(BytesN<32>),
}

/// Signature an agent attaches to authorize a transfer from this account.
#[contracttype]
#[derive(Clone)]
pub struct SessionSignature {
    pub public_key: BytesN<32>,
    pub signature: BytesN<64>,
}

// ------------------------------------------------------------------ events

#[contractevent]
pub struct MandateCreated {
    #[topic]
    pub mandate_id: u64,
    pub session_key: BytesN<32>,
    pub token: Address,
    pub cap_per_epoch: i128,
    pub epoch_secs: u64,
    pub expires_at: u64,
}

#[contractevent]
pub struct MandateRevoked {
    #[topic]
    pub mandate_id: u64,
}

#[contractevent]
pub struct AllowanceConsumed {
    #[topic]
    pub mandate_id: u64,
    pub destination: Address,
    pub amount: i128,
    pub remaining: i128,
}

#[contractevent]
pub struct VerifiedSet {
    #[topic]
    pub subject: Address,
    pub verified: bool,
}

// -------------------------------------------------------------------- TTL

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 30 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 90 * DAY_LEDGERS;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn bump_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

// ---------------------------------------------------------------- helpers

fn owner(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Owner).unwrap()
}

fn is_verified(env: &Env, subject: &Address) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Verified(subject.clone()))
        .unwrap_or(false)
}

fn load_mandate(env: &Env, id: u64) -> Result<Mandate, Error> {
    let key = DataKey::Mandate(id);
    let m = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(Error::UnknownMandate)?;
    bump_persistent(env, &key);
    Ok(m)
}

fn save_mandate(env: &Env, id: u64, m: &Mandate) {
    let key = DataKey::Mandate(id);
    env.storage().persistent().set(&key, m);
    bump_persistent(env, &key);
}

/// Roll the epoch forward if it has elapsed. Pure on `m`.
fn roll_epoch(m: &mut Mandate, now: u64) {
    if now >= m.epoch_start + m.epoch_secs {
        let elapsed = (now - m.epoch_start) / m.epoch_secs;
        m.epoch_start += elapsed * m.epoch_secs;
        m.spent_this_epoch = 0;
    }
}

fn arg<T: TryFromVal<Env, Val>>(env: &Env, args: &Vec<Val>, i: u32) -> Result<T, Error> {
    let v = args.get(i).ok_or(Error::BadArgs)?;
    T::try_from_val(env, &v).map_err(|_| Error::BadArgs)
}

// --------------------------------------------------------------- contract

#[contract]
pub struct AllowanceAccount;

#[contractimpl]
impl AllowanceAccount {
    /// `owner` manages mandates; `issuer` maintains the verified-entity
    /// allowlist (Registry stub).
    pub fn __constructor(env: Env, owner: Address, issuer: Address) {
        env.storage().instance().set(&DataKey::Owner, &owner);
        env.storage().instance().set(&DataKey::Issuer, &issuer);
        env.storage().instance().set(&DataKey::NextId, &0u64);
        bump_instance(&env);
    }

    /// Registry stub. The funded Credential Registry replaces this.
    pub fn set_verified(env: Env, subject: Address, verified: bool) {
        let issuer: Address = env.storage().instance().get(&DataKey::Issuer).unwrap();
        issuer.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Verified(subject.clone()), &verified);
        bump_instance(&env);
        VerifiedSet { subject, verified }.publish(&env);
    }

    pub fn create_mandate(
        env: Env,
        session_key: BytesN<32>,
        token: Address,
        cap_per_epoch: i128,
        epoch_secs: u64,
        expires_at: u64,
        allowed_destinations: Vec<Address>,
    ) -> Result<u64, Error> {
        let owner = owner(&env);
        owner.require_auth();
        if !is_verified(&env, &owner) {
            return Err(Error::NotVerified);
        }
        let now = env.ledger().timestamp();
        if cap_per_epoch <= 0
            || epoch_secs == 0
            || expires_at <= now
            || allowed_destinations.is_empty()
        {
            return Err(Error::InvalidMandate);
        }
        let key_idx = DataKey::KeyIndex(session_key.clone());
        if env.storage().persistent().has(&key_idx) {
            return Err(Error::SessionKeyInUse);
        }

        let id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap();
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        bump_instance(&env);

        let m = Mandate {
            session_key: session_key.clone(),
            token: token.clone(),
            cap_per_epoch,
            epoch_secs,
            epoch_start: now,
            spent_this_epoch: 0,
            expires_at,
            allowed_destinations,
            revoked: false,
        };
        save_mandate(&env, id, &m);
        env.storage().persistent().set(&key_idx, &id);
        bump_persistent(&env, &key_idx);

        MandateCreated {
            mandate_id: id,
            session_key,
            token,
            cap_per_epoch,
            epoch_secs,
            expires_at,
        }
        .publish(&env);
        Ok(id)
    }

    /// Immediate. Every later transfer signed by this mandate's key fails.
    pub fn revoke_mandate(env: Env, mandate_id: u64) -> Result<(), Error> {
        owner(&env).require_auth();
        let mut m = load_mandate(&env, mandate_id)?;
        m.revoked = true;
        save_mandate(&env, mandate_id, &m);
        MandateRevoked { mandate_id }.publish(&env);
        Ok(())
    }

    /// Amount still spendable in the current epoch (0 if revoked/expired).
    pub fn remaining(env: Env, mandate_id: u64) -> Result<i128, Error> {
        let mut m = load_mandate(&env, mandate_id)?;
        let now = env.ledger().timestamp();
        if m.revoked || now >= m.expires_at {
            return Ok(0);
        }
        roll_epoch(&mut m, now);
        Ok(m.cap_per_epoch - m.spent_this_epoch)
    }

    pub fn get_mandate(env: Env, mandate_id: u64) -> Result<Mandate, Error> {
        load_mandate(&env, mandate_id)
    }

    /// Owner moves funds out directly (not mandate-limited).
    pub fn withdraw(env: Env, token: Address, to: Address, amount: i128) {
        owner(&env).require_auth();
        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &to,
            &amount,
        );
    }

    /// Keeper hook: anyone may extend TTL on a live mandate.
    pub fn bump(env: Env, mandate_id: u64) -> Result<(), Error> {
        bump_instance(&env);
        load_mandate(&env, mandate_id).map(|_| ())
    }
}

#[contractimpl]
impl CustomAccountInterface for AllowanceAccount {
    type Signature = SessionSignature;
    type Error = Error;

    #[allow(non_snake_case)]
    fn __check_auth(
        env: Env,
        signature_payload: Hash<32>,
        signature: SessionSignature,
        auth_contexts: Vec<Context>,
    ) -> Result<(), Error> {
        // 1. Which mandate does this key belong to? (Read before verifying
        //    the signature so unknown keys fail with a clear error.)
        let key_idx = DataKey::KeyIndex(signature.public_key.clone());
        let mandate_id: u64 = env
            .storage()
            .persistent()
            .get(&key_idx)
            .ok_or(Error::UnknownSessionKey)?;

        // 2. The session key signed this exact payload. Traps on failure.
        env.crypto().ed25519_verify(
            &signature.public_key,
            &signature_payload.to_bytes().into(),
            &signature.signature,
        );

        let mut m = load_mandate(&env, mandate_id)?;
        let now = env.ledger().timestamp();
        if m.revoked {
            return Err(Error::MandateRevoked);
        }
        if now >= m.expires_at {
            return Err(Error::MandateExpired);
        }
        roll_epoch(&mut m, now);

        // 3. Every authorized call must be `token.transfer(self, dest, amt)`
        //    within the cap. A session key can do nothing else.
        let me = env.current_contract_address();
        for ctx in auth_contexts.iter() {
            let ContractContext {
                contract,
                fn_name,
                args,
            } = match ctx {
                Context::Contract(c) => c,
                _ => return Err(Error::FunctionNotAllowed),
            };
            if contract != m.token {
                return Err(Error::TokenNotAllowed);
            }
            if fn_name != symbol_short!("transfer") {
                return Err(Error::FunctionNotAllowed);
            }
            let from: Address = arg(&env, &args, 0)?;
            let to: Address = arg(&env, &args, 1)?;
            let amount: i128 = arg(&env, &args, 2)?;
            if from != me {
                return Err(Error::BadArgs);
            }
            if amount <= 0 {
                return Err(Error::InvalidAmount);
            }
            if !m.allowed_destinations.contains(&to) {
                return Err(Error::DestinationNotAllowed);
            }
            let spent = m
                .spent_this_epoch
                .checked_add(amount)
                .ok_or(Error::CapExceeded)?;
            if spent > m.cap_per_epoch {
                return Err(Error::CapExceeded);
            }
            m.spent_this_epoch = spent;
            AllowanceConsumed {
                mandate_id,
                destination: to,
                amount,
                remaining: m.cap_per_epoch - spent,
            }
            .publish(&env);
        }

        save_mandate(&env, mandate_id, &m);
        bump_persistent(&env, &key_idx);
        bump_instance(&env);
        Ok(())
    }
}

mod test;
