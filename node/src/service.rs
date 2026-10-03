//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.
//!
//! Consensus wiring per `docs/ghost-consensus-design.md` §9:
//! * Block production is real PoW: `PowBlockImport` verifies every imported
//!   seal via `GhostPowAlgorithm`, and `--mine` spawns `start_mining_worker`
//!   plus `N` grind threads that submit `GhostSeal`s through `MiningHandle`.
//! * Fork choice is heaviest-chain on `PowAux.total_difficulty` via
//!   `HeaviestChain` — the same `SelectChain` feeds the PoW import and the
//!   GRANDPA voter, so finality tracks work, not height.
//! * GRANDPA finalizes with keystore session keys (`KeyTypeId = grandpa`)
//!   elected by `pallet_session`; Aura is gone entirely.

use futures::FutureExt;
use sc_client_api::{Backend, BlockBackend};
use sc_consensus::{BoxBlockImport, JustificationSyncLink};
use sc_consensus_grandpa::SharedVoterState;
use sc_service::{error::Error as ServiceError, Configuration, TaskManager, WarpSyncParams};
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use solochain_template_runtime::{self, apis::RuntimeApi, opaque::Block};
use sp_api::ProvideRuntimeApi;
use sp_core::{crypto::AccountId32, U256};
use sp_runtime::traits::Block as BlockT;

use codec::Encode;
use ghost_consensus::{
    GhostPowAlgorithm, GhostPowApi, HeaviestChain, MiningHandle, PowAlgorithm, PowBlockImport,
};

use std::{sync::Arc, time::Duration};

pub(crate) type FullClient = sc_service::TFullClient<
    Block,
    RuntimeApi,
    sc_executor::WasmExecutor<sp_io::SubstrateHostFunctions>,
>;
type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = HeaviestChain<FullBackend, Block>;
type FullGrandpaBlockImport =
    sc_consensus_grandpa::GrandpaBlockImport<FullBackend, Block, FullClient, FullSelectChain>;

/// The minimum period of blocks on which justifications will be
/// imported and generated.
const GRANDPA_JUSTIFICATION_PERIOD: u32 = 512;

/// Difficulty used when neither the runtime API nor the pow aux store can
/// supply one (genesis parent on a cold start). Matches the runtime's
/// `INITIAL_DIFFICULTY` genesis preset — the runtime API is consulted first,
/// so this is only a last-resort value.
const FALLBACK_INITIAL_DIFFICULTY: u64 = 1_000_000;

/// Milliseconds a grind thread sleeps when the worker has no fresh build to
/// mine on (e.g. the node is still syncing or has just imported a block).
const MINING_IDLE_SLEEP_MS: u64 = 50;

/// Nonces a single `grind` call tries before the loop re-reads
/// `MiningHandle::metadata()` — keeps the loop responsive to new best heads.
const MINING_GRIND_BATCH: u64 = 100_000;

/// PoW authoring configuration resolved from the CLI (`--mine`,
/// `--mining-threads`, `--miner-coinbase`).
#[derive(Clone)]
pub struct MiningConfig {
    /// Whether this node authors PoW blocks.
    pub mine: bool,
    /// Number of grinding threads (>= 1).
    pub threads: usize,
    /// Miner account written into `PreRuntime(POW_ENGINE_ID, _)`. Rewards and
    /// authorship are attributed to it. Must be `Some` when `mine` is set.
    pub coinbase: Option<AccountId32>,
}

/// Timestamp is the only inherent on Ghost (design doc §10 — no slot
/// inherent). Shared by the import-queue inherent check and the proposer.
fn timestamp_inherent_provider(
    _: <Block as BlockT>::Hash,
    _: (),
) -> futures::future::Ready<
    Result<sp_timestamp::InherentDataProvider, Box<dyn std::error::Error + Send + Sync>>,
> {
    futures::future::ready(Ok(sp_timestamp::InherentDataProvider::from_system_time()))
}

/// One grind thread: reads the current `MiningMetadata` from the worker and
/// grinds its own nonce stride (`thread_index` stepping `thread_count`) until
/// the PoW hash meets the per-block difficulty, then submits the seal.
fn mining_thread_loop<B, A, L, Proof>(
    handle: MiningHandle<B, A, L, Proof>,
    thread_index: u64,
    thread_count: u64,
) where
    B: BlockT,
    A: PowAlgorithm<B, Difficulty = U256>,
    A::Difficulty: Send + 'static,
    L: JustificationSyncLink<B>,
{
    let mut last_pre_hash: Option<B::Hash> = None;
    let mut nonce = thread_index;
    loop {
        match handle.metadata() {
            Some(metadata) => {
                // A new best head gives a new pre-hash: restart this thread's
                // stride at its partition offset.
                if last_pre_hash != Some(metadata.pre_hash) {
                    nonce = thread_index;
                    last_pre_hash = Some(metadata.pre_hash);
                }
                if let Some(seal) =
                    ghost_consensus::grind(&metadata, nonce, thread_count, MINING_GRIND_BATCH)
                {
                    if futures::executor::block_on(handle.submit(seal.encode())) {
                        log::info!(
                            target: "pow",
                            "⛏️  submitted winning seal nonce={} for pre_hash={:?}",
                            seal.nonce,
                            metadata.pre_hash,
                        );
                    }
                }
                nonce = nonce.wrapping_add(thread_count.saturating_mul(MINING_GRIND_BATCH));
            }
            None => {
                last_pre_hash = None;
                std::thread::sleep(Duration::from_millis(MINING_IDLE_SLEEP_MS));
            }
        }
    }
}

pub type Service = sc_service::PartialComponents<
    FullClient,
    FullBackend,
    FullSelectChain,
    sc_consensus::DefaultImportQueue<Block>,
    sc_transaction_pool::FullPool<Block, FullClient>,
    (
        // PoW block import (GRANDPA link inside), kept boxed for the mining
        // worker so mined blocks flow through the same verify path.
        BoxBlockImport<Block>,
        GhostPowAlgorithm<Block, FullClient>,
        sc_consensus_grandpa::LinkHalf<Block, FullClient, FullSelectChain>,
        Option<Telemetry>,
    ),
>;

pub fn new_partial(config: &Configuration) -> Result<Service, ServiceError> {
    let telemetry = config
        .telemetry_endpoints
        .clone()
        .filter(|x| !x.is_empty())
        .map(|endpoints| -> Result<_, sc_telemetry::Error> {
            let worker = TelemetryWorker::new(16)?;
            let telemetry = worker.handle().new_telemetry(endpoints);
            Ok((worker, telemetry))
        })
        .transpose()?;

    let executor = sc_service::new_wasm_executor::<sp_io::SubstrateHostFunctions>(config);
    let (client, backend, keystore_container, task_manager) =
        sc_service::new_full_parts::<Block, RuntimeApi, _>(
            config,
            telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
            executor,
        )?;
    let client = Arc::new(client);

    let telemetry = telemetry.map(|(worker, telemetry)| {
        task_manager
            .spawn_handle()
            .spawn("telemetry", None, worker.run());
        telemetry
    });

    // Heaviest-chain fork choice: leaves ordered by PowAux.total_difficulty,
    // tie-broken by number then hash (design doc §9).
    let select_chain = HeaviestChain::new(backend.clone());

    let transaction_pool = sc_transaction_pool::BasicPool::new_full(
        config.transaction_pool.clone(),
        config.role.is_authority().into(),
        config.prometheus_registry(),
        task_manager.spawn_essential_handle(),
        client.clone(),
    );

    let (grandpa_block_import, grandpa_link) = sc_consensus_grandpa::block_import(
        client.clone(),
        GRANDPA_JUSTIFICATION_PERIOD,
        &client,
        select_chain.clone(),
        telemetry.as_ref().map(|x| x.handle()),
    )?;

    // The runtime is the source of truth for difficulty
    // (`GhostPowApi::next_difficulty`); the constant only covers the case
    // where the call is impossible (e.g. genesis on a cold start).
    let initial_difficulty = client
        .block_hash(0)
        .ok()
        .flatten()
        .and_then(|genesis| client.runtime_api().next_difficulty(genesis).ok())
        .unwrap_or_else(|| U256::from(FALLBACK_INITIAL_DIFFICULTY));
    let algorithm = GhostPowAlgorithm::new(client.clone(), initial_difficulty);

    // PoW seal verification first, then the GRANDPA block import handles
    // authority-set tracking and justifications.
    let pow_block_import = PowBlockImport::<
        Block,
        FullGrandpaBlockImport,
        FullClient,
        FullSelectChain,
        GhostPowAlgorithm<Block, FullClient>,
        _,
    >::new(
        grandpa_block_import.clone(),
        client.clone(),
        algorithm.clone(),
        // `check_inherents_after` = 0: every block's inherents are checked.
        0,
        select_chain.clone(),
        timestamp_inherent_provider,
    );

    let import_queue = ghost_consensus::import_queue::<Block, _>(
        Box::new(pow_block_import.clone()),
        Some(Box::new(grandpa_block_import)),
        algorithm.clone(),
        &task_manager.spawn_essential_handle(),
        config.prometheus_registry(),
    )?;

    Ok(sc_service::PartialComponents {
        client,
        backend,
        task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (
            Box::new(pow_block_import) as BoxBlockImport<Block>,
            algorithm,
            grandpa_link,
            telemetry,
        ),
    })
}

/// Builds a new service for a full client.
pub fn new_full<
    N: sc_network::NetworkBackend<Block, <Block as sp_runtime::traits::Block>::Hash>,
>(
    config: Configuration,
    mining: MiningConfig,
) -> Result<TaskManager, ServiceError> {
    let sc_service::PartialComponents {
        client,
        backend,
        mut task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (pow_block_import, algorithm, grandpa_link, mut telemetry),
    } = new_partial(&config)?;

    let mut net_config = sc_network::config::FullNetworkConfiguration::<
        Block,
        <Block as sp_runtime::traits::Block>::Hash,
        N,
    >::new(&config.network);
    let metrics = N::register_notification_metrics(config.prometheus_registry());

    let peer_store_handle = net_config.peer_store_handle();
    let grandpa_protocol_name = sc_consensus_grandpa::protocol_standard_name(
        &client
            .block_hash(0)
            .ok()
            .flatten()
            .expect("Genesis block exists; qed"),
        &config.chain_spec,
    );
    let (grandpa_protocol_config, grandpa_notification_service) =
        sc_consensus_grandpa::grandpa_peers_set_config::<_, N>(
            grandpa_protocol_name.clone(),
            metrics.clone(),
            peer_store_handle,
        );
    net_config.add_notification_protocol(grandpa_protocol_config);

    let warp_sync = Arc::new(sc_consensus_grandpa::warp_proof::NetworkProvider::new(
        backend.clone(),
        grandpa_link.shared_authority_set().clone(),
        Vec::default(),
    ));

    let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
        sc_service::build_network(sc_service::BuildNetworkParams {
            config: &config,
            net_config,
            client: client.clone(),
            transaction_pool: transaction_pool.clone(),
            spawn_handle: task_manager.spawn_handle(),
            import_queue,
            block_announce_validator_builder: None,
            warp_sync_params: Some(WarpSyncParams::WithProvider(warp_sync)),
            block_relay: None,
            metrics,
        })?;

    if config.offchain_worker.enabled {
        task_manager.spawn_handle().spawn(
            "offchain-workers-runner",
            "offchain-worker",
            sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
                runtime_api_provider: client.clone(),
                is_validator: config.role.is_authority(),
                keystore: Some(keystore_container.keystore()),
                offchain_db: backend.offchain_storage(),
                transaction_pool: Some(OffchainTransactionPoolFactory::new(
                    transaction_pool.clone(),
                )),
                network_provider: Arc::new(network.clone()),
                enable_http_requests: true,
                custom_extensions: |_| vec![],
            })
            .run(client.clone(), task_manager.spawn_handle())
            .boxed(),
        );
    }

    let role = config.role.clone();
    let name = config.network.node_name.clone();
    let enable_grandpa = !config.disable_grandpa;
    let prometheus_registry = config.prometheus_registry().cloned();

    let rpc_extensions_builder = {
        let client = client.clone();
        let pool = transaction_pool.clone();

        Box::new(move |deny_unsafe, _| {
            let deps = crate::rpc::FullDeps {
                client: client.clone(),
                pool: pool.clone(),
                deny_unsafe,
            };
            crate::rpc::create_full(deps).map_err(Into::into)
        })
    };

    let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
        network: Arc::new(network.clone()),
        client: client.clone(),
        keystore: keystore_container.keystore(),
        task_manager: &mut task_manager,
        transaction_pool: transaction_pool.clone(),
        rpc_builder: rpc_extensions_builder,
        backend,
        system_rpc_tx,
        tx_handler_controller,
        sync_service: sync_service.clone(),
        config,
        telemetry: telemetry.as_mut(),
    })?;

    network_starter.start_network();

    if mining.mine {
        let coinbase = mining.coinbase.clone().ok_or_else(|| {
            ServiceError::Other(
                "--mine requires --miner-coinbase <SS58> (no default outside --dev)".into(),
            )
        })?;

        let proposer_factory = sc_basic_authorship::ProposerFactory::new(
            task_manager.spawn_handle(),
            client.clone(),
            transaction_pool.clone(),
            prometheus_registry.as_ref(),
            telemetry.as_ref().map(|x| x.handle()),
        );

        // `pre_runtime` takes the RAW payload bytes — the worker wraps them in
        // `DigestItem::PreRuntime(POW_ENGINE_ID, bytes)` itself (design doc §2).
        let (mining_handle, mining_worker) = ghost_consensus::start_mining_worker(
            pow_block_import,
            client.clone(),
            select_chain.clone(),
            algorithm.clone(),
            proposer_factory,
            sync_service.clone(),
            sync_service.clone(),
            Some(ghost_consensus::miner_pre_runtime(&coinbase)),
            timestamp_inherent_provider,
            // Wait this long for a new best block before re-proposing anyway.
            Duration::from_secs(10),
            // Extrinsic execution budget per proposal.
            Duration::from_secs(10),
        );

        task_manager.spawn_essential_handle().spawn_blocking(
            "ghost-pow-worker",
            Some("block-authoring"),
            mining_worker,
        );

        let threads = mining.threads.max(1);
        for thread_id in 0..threads {
            let handle = mining_handle.clone();
            std::thread::Builder::new()
                .name(format!("ghost-miner-{thread_id}"))
                .spawn(move || mining_thread_loop(handle, thread_id as u64, threads as u64))
                .map_err(|e| {
                    ServiceError::Other(format!("failed to spawn mining thread {thread_id}: {e}"))
                })?;
        }
        log::info!(
            target: "pow",
            "⛏️  PoW mining enabled: threads={threads}, coinbase={coinbase}",
        );
    }

    if enable_grandpa {
        // if the node isn't actively participating in consensus then it doesn't
        // need a keystore, regardless of which protocol we use below.
        let keystore = if role.is_authority() {
            Some(keystore_container.keystore())
        } else {
            None
        };

        let grandpa_config = sc_consensus_grandpa::Config {
            // FIXME #1578 make this available through chainspec
            gossip_duration: Duration::from_millis(333),
            justification_generation_period: GRANDPA_JUSTIFICATION_PERIOD,
            name: Some(name),
            observer_enabled: false,
            keystore,
            local_role: role,
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            protocol_name: grandpa_protocol_name,
        };

        // start the full GRANDPA voter
        // NOTE: non-authorities could run the GRANDPA observer protocol, but at
        // this point the full voter should provide better guarantees of block
        // and vote data availability than the observer. The observer has not
        // been tested extensively yet and having most nodes in a network run it
        // could lead to finality stalls.
        let grandpa_config = sc_consensus_grandpa::GrandpaParams {
            config: grandpa_config,
            link: grandpa_link,
            network,
            sync: Arc::new(sync_service),
            notification_service: grandpa_notification_service,
            voting_rule: sc_consensus_grandpa::VotingRulesBuilder::default().build(),
            prometheus_registry,
            shared_voter_state: SharedVoterState::empty(),
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            offchain_tx_pool_factory: OffchainTransactionPoolFactory::new(transaction_pool),
        };

        // the GRANDPA voter task is considered infallible, i.e.
        // if it fails we take down the service with it.
        task_manager.spawn_essential_handle().spawn_blocking(
            "grandpa-voter",
            None,
            sc_consensus_grandpa::run_grandpa_voter(grandpa_config)?,
        );
    }

    Ok(task_manager)
}
