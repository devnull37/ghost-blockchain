use crate::{
    benchmarking::{inherent_benchmark_data, RemarkBuilder, TransferKeepAliveBuilder},
    chain_spec,
    cli::{Cli, GhostCommands, Subcommand},
    service,
};
use frame_benchmarking_cli::{BenchmarkCmd, ExtrinsicFactory, SUBSTRATE_REFERENCE_HARDWARE};
use sc_cli::SubstrateCli;
use sc_service::PartialComponents;
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
        "https://github.com/CoolCreator247/ghost-blockchain".into()
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
        Some(Subcommand::Ghost(cmd)) => handle_ghost_command(cmd),
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

            let target_difficulty = difficulty.unwrap_or(u64::MAX / 1_000_000);

            // Create a sample block header for mining demonstration
            let block_header = MiningBlockHeader {
                number: 1,
                parent_hash: H256::zero(),
                state_root: H256::from_low_u64_be(1),
                extrinsics_root: H256::from_low_u64_be(2),
                difficulty: target_difficulty,
            };

            let miner = Miner::new(*threads, target_difficulty);

            match miner.start(block_header) {
                Some((nonce, stats)) => {
                    println!("\n📦 Local mining result:");
                    println!("   Use this nonce: {}", nonce);
                    println!("   Hashes computed: {}", stats.hashes_computed);
                    println!("   Blocks found: {}", stats.blocks_found);
                    println!("   Hash rate: {:.2} H/s", stats.hash_rate);
                    println!("   Time elapsed: {:.2}s", stats.elapsed_time.as_secs_f64());
                    println!(
                        "   This build does not wire the miner into live block submission yet."
                    );
                }
                None => {
                    println!("\n⚠️  Local mining was interrupted or failed");
                }
            }

            Ok(())
        }
        GhostCommands::Stake { amount, account } => {
            println!("🔒 Staking call preview...");
            println!("   Amount: {} Ghost tokens", amount);
            if let Some(acc) = account {
                println!("   Account: {}", acc);
            } else {
                println!("   Using default account (Alice)");
            }
            println!("   Minimum stake: 1 Ghost token");
            println!("\n📝 The pallet prototype defines stake handling,");
            println!("   but the node does not expose a live staking extrinsic flow yet.");
            Ok(())
        }
        GhostCommands::Unstake { amount, account } => {
            println!("🔓 Unstaking call preview...");
            println!("   Amount: {} Ghost tokens", amount);
            if let Some(acc) = account {
                println!("   Account: {}", acc);
            } else {
                println!("   Using default account (Alice)");
            }
            println!("\n📝 The pallet prototype defines unstake handling,");
            println!("   but the node does not expose a live unstaking extrinsic flow yet.");
            Ok(())
        }
        GhostCommands::Balance { account } => {
            println!("💰 Checking balance and staking information...");
            if let Some(acc) = account {
                println!("   Account: {}", acc);
            } else {
                println!("   Showing default development accounts:");
                println!("\n   Alice:");
                println!("      Address: 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY");
                println!("      Balance: 100 Ghost tokens (genesis)");
                println!("\n   Bob:");
                println!("      Address: 5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty");
                println!("      Balance: 100 Ghost tokens (genesis)");
            }
            println!("\n💡 To check live balance, connect to your running node via:");
            println!("   Polkadot.js Apps UI: https://polkadot.js.org/apps/#/accounts");
            Ok(())
        }
        GhostCommands::Status { detailed } => {
            println!("📊 Ghost Consensus Summary");
            println!("═══════════════════════════════════════════════");
            println!("   Consensus design: Hybrid PoW + PoS + PQC");
            println!("   Live node authoring: Aura");
            println!("   Live finality: GRANDPA");
            println!("   Ghost pallet/runtime: informational prototype only");
            println!("   Block Time: 5 seconds");
            println!("   PoW Algorithm: Enhanced Blake2-256 (ASIC-resistant)");
            println!("   Reward Distribution: 40% miner, 60% stakers (prototype)");
            println!("   Block Reward: 10 Ghost tokens per block (prototype)");

            if *detailed {
                println!("\n📈 Detailed Information:");
                println!("═══════════════════════════════════════════════");
                println!("   Minimum Stake: 1 Ghost token (prototype)");
                println!("   Slashing Conditions:");
                println!("      - Double Signing: 100% stake slash");
                println!("      - Invalid Block: 50% stake slash");
                println!("      - Downtime (>100 blocks): 10% stake slash");
                println!("\n   Phase Flow:");
                println!("      1. PoW Mining - pallet/demo logic");
                println!("      2. PoS Validation - pallet/demo logic");
                println!("      3. Finalization - rewards bookkeeping prototype");
                println!("\n   Network Info:");
                println!("      Chain: Ghost Development Chain");
                println!("      Runtime: FRAME-based (Substrate)");
                println!("      Token: Ghost (GHTM)");
                println!(
                    "      PQC: Dilithium5 support is not wired into the live node RPC surface"
                );
            }

            println!("\n💡 Connect your node to inspect live Aura/GRANDPA state.");
            Ok(())
        }
        GhostCommands::Validators { active_only } => {
            println!("👥 Validator Information");
            println!("═══════════════════════════════════════════════");
            if *active_only {
                println!("   Filter: Active validators only");
            } else {
                println!("   Filter: All validators");
            }
            println!("\n   Default Genesis Validators:");
            println!("      - Alice (5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)");
            println!("      - Bob (5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty)");
            println!("\n💡 Live validator state still comes from the active runtime.");
            Ok(())
        }
    }
}
