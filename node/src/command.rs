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
		"Ghost blockchain node with hybrid PoW + PoS consensus".into()
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
			path =>
				Box::new(chain_spec::ChainSpec::from_json_file(std::path::PathBuf::from(path))?),
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
		},
		Some(Subcommand::CheckBlock(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, import_queue, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, import_queue), task_manager))
			})
		},
		Some(Subcommand::ExportChainSpec(cmd)) => {
			let chain_spec = cli.load_spec(&cmd.chain)?;
			cmd.run(chain_spec)
		},
		Some(Subcommand::ExportBlocks(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, .. } = service::new_partial(&config)?;
				Ok((cmd.run(client, config.database), task_manager))
			})
		},
		Some(Subcommand::ExportState(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, .. } = service::new_partial(&config)?;
				Ok((cmd.run(client, config.chain_spec), task_manager))
			})
		},
		Some(Subcommand::ImportBlocks(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, import_queue, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, import_queue), task_manager))
			})
		},
		Some(Subcommand::PurgeChain(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run(config.database))
		},
		Some(Subcommand::Revert(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, backend, .. } =
					service::new_partial(&config)?;
				let aux_revert = Box::new(|client, _, blocks| {
					sc_consensus_grandpa::revert(client, blocks)?;
					Ok(())
				});
				Ok((cmd.run(client, backend, Some(aux_revert)), task_manager))
			})
		},
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
					},
					BenchmarkCmd::Block(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						cmd.run(client)
					},
					#[cfg(not(feature = "runtime-benchmarks"))]
					BenchmarkCmd::Storage(_) => Err(
						"Storage benchmarking can be enabled with `--features runtime-benchmarks`."
							.into(),
					),
					#[cfg(feature = "runtime-benchmarks")]
					BenchmarkCmd::Storage(cmd) => {
						let PartialComponents { client, backend, .. } =
							service::new_partial(&config)?;
						let db = backend.expose_db();
						let storage = backend.expose_storage();
						let shared_cache = backend.expose_shared_trie_cache();

						cmd.run(config, client, db, storage, shared_cache)
					},
					BenchmarkCmd::Overhead(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						let ext_builder = RemarkBuilder::new(client.clone());

						cmd.run(
							config.chain_spec.name().into(),
							client,
							inherent_benchmark_data()?,
							Vec::new(),
							&ext_builder,
							false,
						)
					},
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
					},
					BenchmarkCmd::Machine(cmd) =>
						cmd.run(&config, SUBSTRATE_REFERENCE_HARDWARE.clone()),
				}
			})
		},
		Some(Subcommand::ChainInfo(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run::<Block>(&config))
		},
		Some(Subcommand::Ghost(cmd)) => {
			let runner = cli.create_runner(&cli.run)?;
			runner.sync_run(|_config| handle_ghost_command(cmd))
		},
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
		},
	}
}

/// Handle Ghost-specific CLI commands
fn handle_ghost_command(cmd: &GhostCommands) -> sc_cli::Result<()> {
	match cmd {
		GhostCommands::Mine { threads, difficulty } => {
			println!("🚀 Starting Ghost PoW mining...");
			println!("   Threads: {}", threads);
			if let Some(diff) = difficulty {
				println!("   Target Difficulty: {}", diff);
			} else {
				println!("   Using default difficulty");
			}
			println!("   Mining with enhanced Blake2 algorithm (ASIC-resistant)");
			// TODO: Implement actual mining logic
			println!("   Mining functionality will be implemented in Phase 5");
			Ok(())
		},
		GhostCommands::Stake { amount, account } => {
			println!("🔒 Staking tokens for PoS validation...");
			println!("   Amount: {} Ghost tokens", amount);
			if let Some(acc) = account {
				println!("   Account: {}", acc);
			} else {
				println!("   Using default account");
			}
			println!("   Minimum stake: 1 Ghost token");
			// TODO: Implement actual staking logic
			println!("   Staking functionality will be implemented in Phase 5");
			Ok(())
		},
		GhostCommands::Unstake { amount, account } => {
			println!("🔓 Unstaking tokens...");
			println!("   Amount: {} Ghost tokens", amount);
			if let Some(acc) = account {
				println!("   Account: {}", acc);
			} else {
				println!("   Using default account");
			}
			// TODO: Implement actual unstaking logic
			println!("   Unstaking functionality will be implemented in Phase 5");
			Ok(())
		},
		GhostCommands::Balance { account } => {
			println!("💰 Checking balance and staking information...");
			if let Some(acc) = account {
				println!("   Account: {}", acc);
			} else {
				println!("   Showing all accounts");
			}
			println!("   Balance queries will be implemented in Phase 5");
			Ok(())
		},
		GhostCommands::Status { detailed } => {
			println!("📊 Ghost Consensus Status");
			println!("   Current Phase: PoW Mining");
			println!("   Block Time: 5 seconds");
			println!("   Consensus: Hybrid PoW + PoS");
			println!("   PoW Algorithm: Enhanced Blake2-256");
			println!("   Reward Distribution: 40% miner, 60% stakers");

			if *detailed {
				println!("\n📈 Detailed Information:");
				println!("   Active Validators: 0");
				println!("   Total Staked: 0 Ghost tokens");
				println!("   Current Difficulty: Default");
				println!("   Blocks Mined: 0");
			}
			Ok(())
		},
		GhostCommands::Validators { active_only } => {
			println!("👥 Validator Information");
			if *active_only {
				println!("   Showing only active validators");
			} else {
				println!("   Showing all validators");
			}
			println!("   No validators currently active");
			println!("   Validator management will be implemented in Phase 5");
			Ok(())
		},
	}
}
