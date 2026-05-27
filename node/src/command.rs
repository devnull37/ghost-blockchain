use crate::{
    benchmarking::{inherent_benchmark_data, RemarkBuilder, TransferKeepAliveBuilder},
    chain_spec,
    cli::{Cli, GhostCommands, Subcommand},
    service,
};
use frame_benchmarking_cli::{BenchmarkCmd, ExtrinsicFactory, SUBSTRATE_REFERENCE_HARDWARE};
use sc_cli::SubstrateCli;
use sc_service::PartialComponents;
use solochain_template_runtime::{Block, EXISTENTIAL_DEPOSIT, UNIT};
use sp_keyring::Sr25519Keyring;

const CLI_STATUS_SUMMARY: &str =
    "Ghost node with classical transport, Proof-of-Work block authoring, longest-chain (PoW) finality, and an experimental Ghost runtime pallet.";
const PQ_READINESS_NOTE: &str =
    "On-chain ML-DSA-87 signature verification is active in the Ghost consensus pallet. An ML-KEM-1024 encryption module exists node-side. libp2p transport remains classical. Any PQ fields are record-only metadata fields: non-enforcing runtime records plus opaque attestation envelopes for claim tracking.";

impl SubstrateCli for Cli {
    fn impl_name() -> String {
        "Ghost Node".into()
    }

    fn impl_version() -> String {
        env!("SUBSTRATE_CLI_IMPL_VERSION").into()
    }

    fn description() -> String {
        format!("{CLI_STATUS_SUMMARY} {PQ_READINESS_NOTE}")
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

/// Parse and run command line arguments
pub fn run() -> sc_cli::Result<()> {
    let cli = Cli::from_args();

    match &cli.subcommand {
        Some(Subcommand::Key(cmd)) => cmd.run(&cli),
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
                // Proof-of-Work has no finality-gadget justifications to revert.
                let aux_revert = Box::new(|_client, _, _blocks| Ok(()));
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
        Some(Subcommand::Ghost(cmd)) => {
            let runner = cli.create_runner(&cli.run)?;
            runner.sync_run(|_config| handle_ghost_command(cmd))
        }
        None => {
            let runner = cli.create_runner(&cli.run)?;
            runner.run_node_until_exit(|config| async move {
                match config.network.network_backend {
					sc_network::config::NetworkBackendType::Libp2p => service::new_full::<
						sc_network::NetworkWorker<
							solochain_template_runtime::opaque::Block,
							<solochain_template_runtime::opaque::Block as sp_runtime::traits::Block>::Hash,
						>,
					>(config)
					.map_err(sc_cli::Error::Service),
					sc_network::config::NetworkBackendType::Litep2p =>
						service::new_full::<sc_network::Litep2pNetworkBackend>(config)
							.map_err(sc_cli::Error::Service),
				}
            })
        }
    }
}

/// Handle Ghost-specific CLI commands
fn handle_ghost_command(cmd: &GhostCommands) -> sc_cli::Result<()> {
    match cmd {
        GhostCommands::Mine {
            threads,
            difficulty,
        } => {
            use crate::miner::{Miner, MiningBlockHeader};
            use sp_core::H256;

            // Conventional difficulty (larger = harder), matching the runtime and
            // crate::pow. The default samples a meaningful hash rate in ~a second.
            let target_difficulty = difficulty.unwrap_or(1_000_000);
            let block_header = MiningBlockHeader {
                number: 1,
                parent_hash: H256::zero(),
                state_root: H256::from_low_u64_be(1),
                extrinsics_root: H256::from_low_u64_be(2),
                difficulty: target_difficulty,
            };
            let miner = Miner::new(*threads as usize, target_difficulty)
                .map_err(|message| sc_cli::Error::Input(message.into()))?;

            match miner.start(block_header) {
                Some((nonce, stats)) => {
                    println!("\nLocal PoW demo result");
                    println!("   Nonce: {}", nonce);
                    println!("   Hashes computed: {}", stats.hashes_computed);
                    println!("   Hash rate: {:.2} H/s", stats.hash_rate);
                    println!("   Elapsed: {:.2}s", stats.elapsed_time.as_secs_f64());
                    println!(
                        "   This command does not submit a block or author a live chain block."
                    );
                }
                None => {
                    println!("\nLocal PoW demo was interrupted or did not find a nonce.");
                }
            }

            Ok(())
        }
        GhostCommands::Stake {
            amount,
            account,
            rpc_url,
        } => {
            let suri = account.as_deref().unwrap_or(crate::wallet::DEFAULT_SURI);
            crate::wallet::stake(*amount, suri, rpc_url.as_deref()).map_err(sc_cli::Error::Input)
        }
        GhostCommands::Unstake {
            amount,
            account,
            rpc_url,
        } => {
            let suri = account.as_deref().unwrap_or(crate::wallet::DEFAULT_SURI);
            crate::wallet::unstake(*amount, suri, rpc_url.as_deref()).map_err(sc_cli::Error::Input)
        }
        GhostCommands::Transfer {
            to,
            amount,
            account,
            rpc_url,
        } => {
            let suri = account.as_deref().unwrap_or(crate::wallet::DEFAULT_SURI);
            crate::wallet::transfer(to, *amount, suri, rpc_url.as_deref())
                .map_err(sc_cli::Error::Input)
        }
        GhostCommands::Balance { account, rpc_url } => {
            let acc = account.as_deref().unwrap_or(crate::wallet::DEFAULT_SURI);
            crate::wallet::show_balance(acc, rpc_url.as_deref()).map_err(sc_cli::Error::Input)
        }
        GhostCommands::Status { detailed } => {
            println!("Ghost Consensus Status");
            println!("===============================================");
            println!("   Network transport: classical libp2p/litep2p stack");
            println!("   Live block authoring: Proof-of-Work (sc-consensus-pow)");
            println!("   Live finality: longest-chain (PoW) selection");
            println!("   Local CLI mining command: local PoW demo only");
            println!("   Experimental pallet model: PoW header submission plus stake-weighted validation");
            println!(
                "   PQ metadata registry: record-only, non-enforcing metadata fields and opaque attestation envelopes"
            );
            println!("   Block Time: 5 seconds");
            println!("   Runtime pallet: ghostConsensus");
            println!("   Demo PoW algorithm: double Blake2-256");
            println!("   Pallet test reward split: 40% miner, 60% stakers");
            println!("   Pallet test block reward: {} raw units", 10 * UNIT);
            println!("   On-chain PQ signatures: ML-DSA-87 signature verification active in Ghost consensus pallet");
            println!("   Node-side PQ encryption module: ML-KEM-1024 (libp2p transport remains classical)");

            if *detailed {
                println!("\nDetailed Information:");
                println!("===============================================");
                println!("   Minimum Stake: {} raw units", UNIT);
                println!("   Slashing Conditions:");
                println!("      - Double Signing: configured slash");
                println!("      - Invalid Block: configured slash");
                println!("      - Downtime (>100 blocks): 10% stake slash");
                println!("\n   PQ Metadata Registry:");
                println!(
                    "      - `ghostConsensus.register_pq_readiness(...)` stores record-only PQ metadata fields."
                );
                println!(
                    "      - `ghostConsensus.attest_pq_readiness(...)` stores opaque attestation envelopes only."
                );
                println!(
                    "      - `ghostConsensus.remove_pq_readiness()` clears that non-enforcing metadata record and its opaque attestations."
                );
                println!(
                    "      - These extrinsics do not activate live PQ transport, PQ block authoring, PQ finality, or quantum encryption."
                );
                println!("\n   Important Caveat:");
                println!("      - Networking remains on the standard classical transport stack.");
                println!("      - Live blocks are authored via Proof-of-Work (sc-consensus-pow) and finalized by longest-chain selection.");
                println!("      - `ghost mine` is a local demo and does not submit headers or author blocks.");
                println!("      - The Ghost pallet is validated with tests and CLI guidance, not wired into node authoring.");
                println!("      - PQ registry entries are non-enforcing metadata records, and attestations are opaque envelopes, not proof of live PQ enforcement.");
                println!("      - ML-DSA-87 signature verification is active on-chain in the Ghost consensus pallet.");
                println!("      - ML-KEM-1024 encryption module exists node-side; quantum key exchange and PQ session setup are not active on libp2p transport.");
                println!("\n   Pallet Flow:");
                println!("      1. PoW Header Submission - a nonce is produced off-chain");
                println!("      2. Stake-Weighted Validation - validators approve submitted work");
                println!("      3. Finalization - pallet state advances after validation");
                println!("\n   Network Info:");
                println!("      Chain: Ghost Development Chain");
                println!("      Runtime: FRAME-based (Substrate)");
                println!("      Balance unit: 1 Ghost = {} raw units", UNIT);
            }

            println!("\nConnect your node to inspect live state via Polkadot.js Apps.");
            Ok(())
        }
        GhostCommands::Validators {
            active_only: _,
            rpc_url,
        } => crate::wallet::list_validators(rpc_url.as_deref()).map_err(sc_cli::Error::Input),
    }
}
