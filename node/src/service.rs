//! Service and ServiceFactory implementation — Ghost Proof-of-Work edition.
//!
//! Block authoring is real Proof-of-Work via `sc-consensus-pow`. There is no Aura slot
//! scheduler and no GRANDPA finality gadget: the canonical chain is the one with the most
//! accumulated PoW (longest-/heaviest-chain). Difficulty is read from the runtime
//! (`sp_consensus_pow::DifficultyApi`), where `pallet-ghost-consensus` retargets it toward
//! the target block time. The sc-service scaffolding here mirrors the stable2407 solochain
//! template; only the consensus engine differs.

use std::{sync::Arc, time::Duration};

use codec::Encode;
use futures::FutureExt;
use sc_client_api::Backend;
use sc_consensus::BoxBlockImport;
use sc_service::{error::Error as ServiceError, Configuration, TaskManager};
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use solochain_template_runtime::{self, apis::RuntimeApi, opaque::Block};

use crate::pow::{meets_difficulty, pow_hash, GhostPow, GhostSeal};

pub(crate) type FullClient = sc_service::TFullClient<
    Block,
    RuntimeApi,
    sc_executor::WasmExecutor<sp_io::SubstrateHostFunctions>,
>;
type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;

pub type Service = sc_service::PartialComponents<
    FullClient,
    FullBackend,
    FullSelectChain,
    sc_consensus::DefaultImportQueue<Block>,
    sc_transaction_pool::FullPool<Block, FullClient>,
    (
        BoxBlockImport<Block>,
        GhostPow<FullClient>,
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

    let select_chain = sc_consensus::LongestChain::new(backend.clone());

    let transaction_pool = sc_transaction_pool::BasicPool::new_full(
        config.transaction_pool.clone(),
        config.role.is_authority().into(),
        config.prometheus_registry(),
        task_manager.spawn_essential_handle(),
        client.clone(),
    );

    let algorithm = GhostPow::new(client.clone());

    let pow_block_import = sc_consensus_pow::PowBlockImport::new(
        client.clone(),
        client.clone(),
        algorithm.clone(),
        0u32,
        select_chain.clone(),
        |_parent, ()| async {
            let timestamp = sp_timestamp::InherentDataProvider::from_system_time();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(timestamp)
        },
    );

    let import_queue = sc_consensus_pow::import_queue(
        Box::new(pow_block_import.clone()),
        None,
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
        other: (Box::new(pow_block_import), algorithm, telemetry),
    })
}

/// Builds a new service for a full client.
pub fn new_full<
    N: sc_network::NetworkBackend<Block, <Block as sp_runtime::traits::Block>::Hash>,
>(
    config: Configuration,
) -> Result<TaskManager, ServiceError> {
    let sc_service::PartialComponents {
        client,
        backend,
        mut task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (pow_block_import, algorithm, mut telemetry),
    } = new_partial(&config)?;

    let mut net_config = sc_network::config::FullNetworkConfiguration::<
        Block,
        <Block as sp_runtime::traits::Block>::Hash,
        N,
    >::new(&config.network);
    let metrics = N::register_notification_metrics(config.prometheus_registry());

    let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
        sc_service::build_network(sc_service::BuildNetworkParams {
            config: &config,
            net_config,
            client: client.clone(),
            transaction_pool: transaction_pool.clone(),
            spawn_handle: task_manager.spawn_handle(),
            import_queue,
            block_announce_validator_builder: None,
            warp_sync_params: None,
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

    if role.is_authority() {
        let proposer_factory = sc_basic_authorship::ProposerFactory::new(
            task_manager.spawn_handle(),
            client.clone(),
            transaction_pool.clone(),
            prometheus_registry.as_ref(),
            telemetry.as_ref().map(|x| x.handle()),
        );

        let (mining_handle, mining_task) = sc_consensus_pow::start_mining_worker(
            pow_block_import,
            client.clone(),
            select_chain,
            algorithm,
            proposer_factory,
            sync_service.clone(),
            sync_service.clone(),
            None,
            |_parent, ()| async {
                let timestamp = sp_timestamp::InherentDataProvider::from_system_time();
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(timestamp)
            },
            Duration::from_secs(10),
            Duration::from_secs(10),
        );

        task_manager.spawn_essential_handle().spawn_blocking(
            "pow-mining-worker",
            Some("pow"),
            mining_task,
        );

        // CPU miner threads run OFF the async executor: the mining worker's `submit`
        // holds a non-Send lock across an `.await`, so it cannot be a Send async task.
        // Each thread drives `submit` to completion with `block_on`, and searches a
        // disjoint slice of the nonce space (offset by thread id, stepping by the
        // thread count) so threads never duplicate work.
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        for thread_id in 0..threads {
            let mining_handle = mining_handle.clone();
            let step = threads as u64;
            std::thread::Builder::new()
                .name(format!("ghost-pow-miner-{thread_id}"))
                .spawn(move || {
                    let mut current_pre: Option<sp_core::H256> = None;
                    let mut nonce = thread_id as u64;
                    loop {
                        let metadata = match mining_handle.metadata() {
                            Some(metadata) => metadata,
                            None => {
                                std::thread::sleep(Duration::from_millis(300));
                                continue;
                            }
                        };

                        if current_pre != Some(metadata.pre_hash) {
                            current_pre = Some(metadata.pre_hash);
                            nonce = thread_id as u64;
                        }

                        let mut solved = None;
                        for _ in 0..50_000u64 {
                            if meets_difficulty(
                                &pow_hash(&metadata.pre_hash, nonce),
                                metadata.difficulty,
                            ) {
                                solved = Some(nonce);
                                break;
                            }
                            nonce = nonce.wrapping_add(step);
                        }

                        if let Some(found) = solved {
                            let seal = GhostSeal { nonce: found }.encode();
                            let _ = futures::executor::block_on(mining_handle.submit(seal));
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                })
                .expect("ghost PoW miner thread spawns");
        }
    }

    network_starter.start_network();
    Ok(task_manager)
}
