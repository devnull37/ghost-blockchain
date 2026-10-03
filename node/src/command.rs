use crate::{
    benchmarking::{inherent_benchmark_data, RemarkBuilder, TransferKeepAliveBuilder},
    chain_spec,
    cli::{Cli, GhostCommands, MiningCmd, Subcommand},
    service,
};
use frame_benchmarking_cli::{BenchmarkCmd, ExtrinsicFactory, SUBSTRATE_REFERENCE_HARDWARE};
use sc_cli::SubstrateCli;
use sc_service::{ChainType, PartialComponents};
use solochain_template_runtime::{Block, EXISTENTIAL_DEPOSIT};
use sp_keyring::Sr25519Keyring;

impl SubstrateCli for Cli {
    fn impl_name() -> String {
        "Ghost Node".into()
    }

    fn impl_version() -> String {
        env!("SUBSTRATE_CLI_IMPL_VERSION").into()
    }

    fn description() -> String {
        "Ghost blockchain node".into()
    }

    fn author() -> String {
        "Ghost Blockchain Team".into()
    }

    fn support_url() -> String {
        "https://github.com/devnull37/ghost-blockchain".into()
    }

    fn copyright_start_year() -> i32 {
        2025
    }

    fn load_spec(&self, id: &str) -> Result<Box<dyn sc_service::ChainSpec>, String> {
        Ok(match id {
            "dev" => Box::new(chain_spec::development_chain_spec()?),
            "" | "local" => Box::new(chain_spec::local_chain_spec()?),
            path => Box::new(chain_spec::ChainSpec::from_json_file(
                std::path::PathBuf::from(path),
            )?),
        })
    }
}

/// Resolve the `--mine`/`--mining-threads`/`--miner-coinbase` flags into a
/// `MiningConfig`. `--miner-coinbase` is required with `--mine` outside
/// development chains, where it defaults to Alice (design doc §9).
fn resolve_mining_config(
    flags: &MiningCmd,
    config: &sc_service::Configuration,
) -> sc_cli::Result<service::MiningConfig> {
    let threads = flags
        .mining_threads
        .or_else(|| std::thread::available_parallelism().map(|n| n.get()).ok())
        .unwrap_or(1)
        .max(1);

    if !flags.mine {
        return Ok(service::MiningConfig {
            mine: false,
            threads,
            coinbase: None,
        });
    }

    let coinbase = match flags.miner_coinbase.as_deref() {
        Some(ss58) => sp_core::crypto::Ss58Codec::from_ss58check(ss58)
            .map_err(|e| sc_cli::Error::Input(format!("invalid --miner-coinbase '{ss58}': {e}")))?,
        None => {
            if config.chain_spec.chain_type() == ChainType::Development {
                Sr25519Keyring::Alice.to_account_id()
            } else {
                return Err(sc_cli::Error::Input(
                    "--mine requires --miner-coinbase <SS58> (only --dev defaults to Alice)".into(),
                ));
            }
        }
    };

    Ok(service::MiningConfig {
        mine: true,
        threads,
        coinbase: Some(coinbase),
    })
}

/// `ghost verify-pow <block-hash>`: re-verify a block's PoW seal against the
/// local chain database (the block must have been imported by this node).
fn verify_pow(client: std::sync::Arc<service::FullClient>, block_hash: &str) -> sc_cli::Result<()> {
    use codec::Decode;
    use ghost_consensus::GhostPowApi;
    use sp_api::ProvideRuntimeApi;
    use sp_runtime::traits::Header as HeaderT;

    let hash: sp_core::H256 = block_hash
        .parse()
        .map_err(|_| sc_cli::Error::Input(format!("invalid block hash '{block_hash}'")))?;

    let header = client
        .header(hash)
        .map_err(sc_cli::Error::Client)?
        .ok_or_else(|| {
            sc_cli::Error::Input(format!("block {hash} not found in the local database"))
        })?;

    // Same unsealing the import path applies: the trailing digest item must be
    // `Seal(POW_ENGINE_ID, GhostSeal)`, and `pre_hash` is the header hash
    // without it.
    let mut unsealed = header.clone();
    let seal_bytes = match unsealed.digest_mut().pop() {
        Some(sp_runtime::DigestItem::Seal(engine, bytes))
            if engine == ghost_consensus::POW_ENGINE_ID =>
        {
            bytes
        }
        _ => {
            return Err(sc_cli::Error::Input(format!(
                "block {hash} is unsealed or sealed by another engine"
            )))
        }
    };
    let pre_hash = unsealed.hash();

    let pre_digest = unsealed.digest().logs().iter().find_map(|log| match log {
        sp_runtime::DigestItem::PreRuntime(engine, bytes)
            if engine == &ghost_consensus::POW_ENGINE_ID =>
        {
            Some(bytes.clone())
        }
        _ => None,
    });

    let seal = ghost_consensus::GhostSeal::decode(&mut &seal_bytes[..])
        .map_err(|e| sc_cli::Error::Input(format!("malformed seal on {hash}: {e}")))?;
    let author = ghost_consensus::author_from_header(&header);

    let difficulty = client
        .runtime_api()
        .next_difficulty(*unsealed.parent_hash())
        .map_err(|e| sc_cli::Error::Application(e.into()))?;

    let valid = ghost_consensus::verify_seal(
        pre_hash.as_ref(),
        pre_digest.as_deref(),
        &seal_bytes,
        difficulty,
    );

    let number = *unsealed.number();
    println!("block:    #{number} {hash}");
    println!(
        "miner:    {}",
        author
            .map(|a| a.to_string())
            .unwrap_or_else(|| "<none>".into())
    );
    println!("nonce:    {}", seal.nonce);
    println!("work factor (difficulty): {difficulty}");
    println!("pow seal: {}", if valid { "VALID" } else { "INVALID" });

    if valid {
        Ok(())
    } else {
        Err(sc_cli::Error::Input(format!(
            "PoW seal on block {hash} is INVALID"
        )))
    }
}

/// Parse and run command line arguments
pub fn run() -> sc_cli::Result<()> {
    let cli = Cli::from_args();

    match &cli.subcommand {
        Some(Subcommand::Key(cmd)) => cmd.run(&cli),
        #[allow(deprecated)]
        Some(Subcommand::BuildSpec(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run(config.chain_spec, config.network))
        }
        Some(Subcommand::CheckBlock(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    import_queue,
                    ..
                } = service::new_partial(&config)?;
                Ok((cmd.run(client, import_queue), task_manager))
            })
        }
        Some(Subcommand::ExportBlocks(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    ..
                } = service::new_partial(&config)?;
                Ok((cmd.run(client, config.database), task_manager))
            })
        }
        Some(Subcommand::ExportState(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    ..
                } = service::new_partial(&config)?;
                Ok((cmd.run(client, config.chain_spec), task_manager))
            })
        }
        Some(Subcommand::ImportBlocks(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    import_queue,
                    ..
                } = service::new_partial(&config)?;
                Ok((cmd.run(client, import_queue), task_manager))
            })
        }
        Some(Subcommand::PurgeChain(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run(config.database))
        }
        Some(Subcommand::Revert(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    backend,
                    ..
                } = service::new_partial(&config)?;
                let aux_revert = Box::new(|client, _, blocks| {
                    sc_consensus_grandpa::revert(client, blocks)?;
                    Ok(())
                });
                Ok((cmd.run(client, backend, Some(aux_revert)), task_manager))
            })
        }
        Some(Subcommand::Benchmark(cmd)) => {
            let runner = cli.create_runner(cmd)?;

            runner.sync_run(|config| {
                // This switch needs to be in the client, since the client decides
                // which sub-commands it wants to support.
                match cmd {
                    BenchmarkCmd::Pallet(cmd) => {
                        if !cfg!(feature = "runtime-benchmarks") {
                            return Err(
                                "Runtime benchmarking wasn't enabled when building the node. \
							You can enable it with `--features runtime-benchmarks`."
                                    .into(),
                            );
                        }

                        cmd.run_with_spec::<sp_runtime::traits::HashingFor<Block>, ()>(Some(
                            config.chain_spec,
                        ))
                    }
                    BenchmarkCmd::Block(cmd) => {
                        let PartialComponents { client, .. } = service::new_partial(&config)?;
                        cmd.run(client)
                    }
                    #[cfg(not(feature = "runtime-benchmarks"))]
                    BenchmarkCmd::Storage(_) => Err(
                        "Storage benchmarking can be enabled with `--features runtime-benchmarks`."
                            .into(),
                    ),
                    #[cfg(feature = "runtime-benchmarks")]
                    BenchmarkCmd::Storage(cmd) => {
                        let PartialComponents {
                            client, backend, ..
                        } = service::new_partial(&config)?;
                        let db = backend.expose_db();
                        let storage = backend.expose_storage();
                        cmd.run(config, client, db, storage)
                    }
                    BenchmarkCmd::Overhead(cmd) => {
                        let PartialComponents { client, .. } = service::new_partial(&config)?;
                        let ext_builder = RemarkBuilder::new(client.clone());

                        cmd.run(
                            config,
                            client,
                            inherent_benchmark_data()?,
                            Vec::new(),
                            &ext_builder,
                        )
                    }
                    BenchmarkCmd::Extrinsic(cmd) => {
                        let PartialComponents { client, .. } = service::new_partial(&config)?;
                        // Register the *Remark* and *TKA* builders.
                        let ext_factory = ExtrinsicFactory(vec![
                            Box::new(RemarkBuilder::new(client.clone())),
                            Box::new(TransferKeepAliveBuilder::new(
                                client.clone(),
                                Sr25519Keyring::Alice.to_account_id(),
                                EXISTENTIAL_DEPOSIT,
                            )),
                        ]);

                        cmd.run(client, inherent_benchmark_data()?, Vec::new(), &ext_factory)
                    }
                    BenchmarkCmd::Machine(cmd) => {
                        cmd.run(&config, SUBSTRATE_REFERENCE_HARDWARE.clone())
                    }
                }
            })
        }
        Some(Subcommand::ChainInfo(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run::<Block>(&config))
        }
        Some(Subcommand::Ghost(GhostCommands::VerifyPow(cmd))) => {
            let runner = cli.create_runner(cmd)?;
            let block_hash = cmd.block_hash.clone();
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    ..
                } = service::new_partial(&config)?;
                Ok((async move { verify_pow(client, &block_hash) }, task_manager))
            })
        }
        None => {
            let runner = cli.create_runner(&cli.run)?;
            let mining_flags = cli.mining.clone();
            runner.run_node_until_exit(move |config| async move {
                let mining = resolve_mining_config(&mining_flags, &config)?;
                match config.network.network_backend {
					sc_network::config::NetworkBackendType::Libp2p => service::new_full::<
						sc_network::NetworkWorker<
							solochain_template_runtime::opaque::Block,
							<solochain_template_runtime::opaque::Block as sp_runtime::traits::Block>::Hash,
						>,
					>(config, mining)
					.map_err(sc_cli::Error::Service),
					sc_network::config::NetworkBackendType::Litep2p =>
						service::new_full::<sc_network::Litep2pNetworkBackend>(config, mining)
							.map_err(sc_cli::Error::Service),
				}
            })
        }
    }
}
