//! Minimal RPC wallet / transactor for the Ghost node CLI.
//!
//! Connects to a running node's JSON-RPC endpoint (default `http://127.0.0.1:9944`) to:
//! - query live balances and the on-chain validator/stake set (read-only), and
//! - build, sign (sr25519), and submit real extrinsics (`stake`, `unstake`, `transfer`)
//!   using the runtime's own `RuntimeCall`/`UncheckedExtrinsic`/`TxExtension` types, so the
//!   SCALE encoding always matches the runtime this binary was built from.
//!
//! Signing uses `SignedPayload::from_raw` with the additional-signed data supplied locally
//! (spec/tx version from the embedded `VERSION`, genesis hash fetched over RPC), so it needs
//! no local chain state. Era is immortal and the metadata-hash extension is disabled, which
//! matches a node run without the `metadata-hash` feature.

use codec::{Decode, Encode};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use sp_core::{
    blake2_128,
    bytes::{from_hex, to_hex},
    crypto::{Pair, Ss58Codec},
    sr25519, twox_128, H256,
};
use sp_runtime::{
    generic::Era, traits::IdentifyAccount, MultiAddress, MultiSignature, MultiSigner,
};

use solochain_template_runtime::{
    AccountId, Balance, Runtime, RuntimeCall, SignedPayload, TxExtension, UncheckedExtrinsic,
    VERSION,
};

/// Default node RPC endpoint (Substrate serves HTTP + WS on the same port).
pub const DEFAULT_RPC: &str = "http://127.0.0.1:9944";
/// Default signer when none is given (development account).
pub const DEFAULT_SURI: &str = "//Alice";

/// Local mirror of `frame_system::AccountInfo<u32, pallet_balances::AccountData<Balance>>`
/// so the `System.Account` storage value can be decoded without extra type plumbing.
#[derive(Decode)]
struct AccountData {
    free: Balance,
    reserved: Balance,
    frozen: Balance,
    _flags: u128,
}

#[derive(Decode)]
struct AccountInfo {
    nonce: u32,
    _consumers: u32,
    _providers: u32,
    _sufficients: u32,
    data: AccountData,
}

fn run<F: core::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to start a tokio runtime for the RPC client")
        .block_on(f)
}

fn connect(rpc: &str) -> Result<HttpClient, String> {
    HttpClientBuilder::default()
        .build(rpc)
        .map_err(|e| format!("failed to connect to node RPC at {rpc}: {e}"))
}

/// Derive (pair, account) from a secret URI / dev seed such as `//Alice` or a mnemonic.
fn signer(suri: &str) -> Result<(sr25519::Pair, AccountId), String> {
    let pair = sr25519::Pair::from_string(suri, None)
        .map_err(|e| format!("invalid signer secret '{suri}': {e:?}"))?;
    let account = MultiSigner::Sr25519(pair.public()).into_account();
    Ok((pair, account))
}

/// Resolve an account from an SS58 address, or from a secret URI / dev seed.
fn resolve_account(s: &str) -> Result<AccountId, String> {
    if let Ok(acc) = AccountId::from_ss58check(s) {
        return Ok(acc);
    }
    let pair = sr25519::Pair::from_string(s, None)
        .map_err(|e| format!("'{s}' is not a valid SS58 address or secret URI: {e:?}"))?;
    Ok(MultiSigner::Sr25519(pair.public()).into_account())
}

fn map_prefix(pallet: &str, item: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(32);
    k.extend_from_slice(&twox_128(pallet.as_bytes()));
    k.extend_from_slice(&twox_128(item.as_bytes()));
    k
}

/// Storage key for a `Blake2_128Concat` map entry (used by both `System.Account` and
/// `GhostConsensus.ValidatorStakes`).
fn blake2_map_key(pallet: &str, item: &str, raw_key: &[u8]) -> Vec<u8> {
    let mut k = map_prefix(pallet, item);
    k.extend_from_slice(&blake2_128(raw_key));
    k.extend_from_slice(raw_key);
    k
}

// ---------------------------------------------------------------------------
// Async RPC primitives
// ---------------------------------------------------------------------------

async fn rpc_next_nonce(c: &HttpClient, account: &AccountId) -> Result<u32, String> {
    c.request("system_accountNextIndex", rpc_params![account.to_ss58check()])
        .await
        .map_err(|e| format!("system_accountNextIndex failed: {e}"))
}

async fn rpc_genesis_hash(c: &HttpClient) -> Result<H256, String> {
    let hex: String = c
        .request("chain_getBlockHash", rpc_params![0u32])
        .await
        .map_err(|e| format!("chain_getBlockHash(0) failed: {e}"))?;
    Ok(H256::from_slice(&from_hex(&hex).map_err(|e| format!("bad genesis hash: {e:?}"))?))
}

async fn rpc_get_storage(c: &HttpClient, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let res: Option<String> = c
        .request("state_getStorage", rpc_params![to_hex(key, false)])
        .await
        .map_err(|e| format!("state_getStorage failed: {e}"))?;
    match res {
        Some(h) => Ok(Some(from_hex(&h).map_err(|e| format!("bad storage hex: {e:?}"))?)),
        None => Ok(None),
    }
}

async fn rpc_keys_paged(c: &HttpClient, prefix: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let prefix_hex = to_hex(prefix, false);
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut start: Option<String> = None;
    loop {
        let page: Vec<String> = c
            .request(
                "state_getKeysPaged",
                rpc_params![prefix_hex.clone(), 256u32, start.clone()],
            )
            .await
            .map_err(|e| format!("state_getKeysPaged failed: {e}"))?;
        let count = page.len();
        start = page.last().cloned();
        for k in page {
            out.push(from_hex(&k).map_err(|e| format!("bad key hex: {e:?}"))?);
        }
        if count < 256 {
            break;
        }
    }
    Ok(out)
}

async fn rpc_submit(c: &HttpClient, xt: &UncheckedExtrinsic) -> Result<H256, String> {
    let hex: String = c
        .request("author_submitExtrinsic", rpc_params![to_hex(&xt.encode(), false)])
        .await
        .map_err(|e| format!("the node rejected the transaction: {e}"))?;
    Ok(H256::from_slice(&from_hex(&hex).map_err(|e| format!("bad tx hash: {e:?}"))?))
}

/// Build a signed extrinsic using the runtime's own extension types. `additional` mirrors
/// each extension's implicit data; `SignedPayload::from_raw` avoids needing chain state.
fn build_signed(
    call: RuntimeCall,
    pair: &sr25519::Pair,
    account: &AccountId,
    nonce: u32,
    genesis: H256,
) -> Result<UncheckedExtrinsic, String> {
    let extra: TxExtension = (
        frame_system::CheckNonZeroSender::<Runtime>::new(),
        frame_system::CheckSpecVersion::<Runtime>::new(),
        frame_system::CheckTxVersion::<Runtime>::new(),
        frame_system::CheckGenesis::<Runtime>::new(),
        frame_system::CheckEra::<Runtime>::from(Era::Immortal),
        frame_system::CheckNonce::<Runtime>::from(nonce),
        frame_system::CheckWeight::<Runtime>::new(),
        pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
        frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
    );

    let additional = (
        (),
        VERSION.spec_version,
        VERSION.transaction_version,
        genesis,
        genesis, // immortal era anchors at the genesis hash
        (),
        (),
        (),
        None::<[u8; 32]>, // metadata-hash extension disabled
    );

    let payload = SignedPayload::from_raw(call.clone(), extra.clone(), additional);
    let signature = payload.using_encoded(|p| pair.sign(p));

    Ok(UncheckedExtrinsic::new_signed(
        call,
        MultiAddress::Id(account.clone()),
        MultiSignature::Sr25519(signature),
        extra,
    ))
}

// ---------------------------------------------------------------------------
// Public command entry points (sync wrappers used by the CLI handlers)
// ---------------------------------------------------------------------------

/// Print live free/reserved/frozen balance and nonce for an account.
pub fn show_balance(account: &str, rpc: Option<&str>) -> Result<(), String> {
    let rpc = rpc.unwrap_or(DEFAULT_RPC).to_string();
    let acc = resolve_account(account)?;
    run(async move {
        let c = connect(&rpc)?;
        let key = blake2_map_key("System", "Account", &acc.encode());
        println!("Account: {}", acc.to_ss58check());
        match rpc_get_storage(&c, &key).await? {
            Some(bytes) => {
                let info = AccountInfo::decode(&mut &bytes[..])
                    .map_err(|e| format!("failed to decode account info: {e}"))?;
                println!("  Free:     {}", info.data.free);
                println!("  Reserved: {}", info.data.reserved);
                println!("  Frozen:   {}", info.data.frozen);
                println!("  Nonce:    {}", info.nonce);
            }
            None => println!("  (no on-chain data; account is empty / never used)"),
        }
        Ok(())
    })
}

/// List the live validator set (accounts with a stake in `GhostConsensus.ValidatorStakes`).
pub fn list_validators(rpc: Option<&str>) -> Result<(), String> {
    let rpc = rpc.unwrap_or(DEFAULT_RPC).to_string();
    run(async move {
        let c = connect(&rpc)?;
        let prefix = map_prefix("GhostConsensus", "ValidatorStakes");
        let keys = rpc_keys_paged(&c, &prefix).await?;
        if keys.is_empty() {
            println!("No validators are currently staked.");
            return Ok(());
        }
        println!("Staked validators ({}):", keys.len());
        for key in keys {
            if key.len() < 32 {
                continue;
            }
            // Key layout: 16-byte twox(pallet) + 16-byte twox(item) + 16-byte blake2 + AccountId.
            let acc = AccountId::decode(&mut &key[key.len() - 32..])
                .map_err(|e| format!("failed to decode validator account: {e}"))?;
            let stake = match rpc_get_storage(&c, &key).await? {
                Some(b) => Balance::decode(&mut &b[..]).unwrap_or(0),
                None => 0,
            };
            println!("  {}  stake {}", acc.to_ss58check(), stake);
        }
        Ok(())
    })
}

fn submit_call(call: RuntimeCall, suri: &str, rpc: Option<&str>, what: &str) -> Result<(), String> {
    let rpc = rpc.unwrap_or(DEFAULT_RPC).to_string();
    let (pair, account) = signer(suri)?;
    run(async move {
        let c = connect(&rpc)?;
        let nonce = rpc_next_nonce(&c, &account).await?;
        let genesis = rpc_genesis_hash(&c).await?;
        let xt = build_signed(call, &pair, &account, nonce, genesis)?;
        let hash = rpc_submit(&c, &xt).await?;
        println!("Submitted {what}");
        println!("  signer:         {}", account.to_ss58check());
        println!("  nonce:          {nonce}");
        println!("  extrinsic hash: {hash:?}");
        println!("  The node has accepted it into the pool; watch its logs for block inclusion.");
        Ok(())
    })
}

/// Sign + submit `ghostConsensus.stake(amount)`.
pub fn stake(amount: u128, suri: &str, rpc: Option<&str>) -> Result<(), String> {
    let call = RuntimeCall::GhostConsensus(pallet_ghost_consensus::Call::stake { amount });
    submit_call(call, suri, rpc, &format!("stake({amount})"))
}

/// Sign + submit `ghostConsensus.unstake(amount)`.
pub fn unstake(amount: u128, suri: &str, rpc: Option<&str>) -> Result<(), String> {
    let call = RuntimeCall::GhostConsensus(pallet_ghost_consensus::Call::unstake { amount });
    submit_call(call, suri, rpc, &format!("unstake({amount})"))
}

/// Sign + submit `balances.transfer_keep_alive(dest, amount)`.
pub fn transfer(dest: &str, amount: u128, suri: &str, rpc: Option<&str>) -> Result<(), String> {
    let dest_acc = resolve_account(dest)?;
    let call = RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive {
        dest: MultiAddress::Id(dest_acc.clone()),
        value: amount,
    });
    submit_call(
        call,
        suri,
        rpc,
        &format!("transfer({amount}) to {}", dest_acc.to_ss58check()),
    )
}
