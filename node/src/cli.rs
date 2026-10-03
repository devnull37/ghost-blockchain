#[derive(Debug, clap::Parser)]
#[command(name = "ghost-node", about = "Ghost blockchain node", version, author)]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,

    #[clap(flatten)]
    pub run: sc_cli::RunCmd,

    /// PoW mining flags (see `docs/ghost-consensus-design.md` §9).
    #[clap(flatten)]
    pub mining: MiningCmd,
}

/// PoW authoring flags for `ghost-node`.
///
/// `--mine` turns this node into a block author: `start_mining_worker`
/// proposes blocks on the heaviest chain and `--mining-threads` grind threads
/// race to seal them. `--miner-coinbase` is the `AccountId32` written into the
/// `PreRuntime(POW_ENGINE_ID, _)` digest — the unspoofable miner attribution
/// the pallet pays block rewards to.
#[derive(Debug, Clone, clap::Args)]
pub struct MiningCmd {
    /// Author PoW blocks (grind seals and submit them for import).
    #[arg(long)]
    pub mine: bool,

    /// Number of PoW grinding threads. Defaults to all available CPU cores.
    #[arg(long, value_name = "N", requires = "mine")]
    pub mining_threads: Option<usize>,

    /// SS58 account recorded as the miner of authored blocks (the reward
    /// coinbase). Required with `--mine`; under `--dev` it defaults to Alice.
    #[arg(long, value_name = "SS58", requires = "mine")]
    pub miner_coinbase: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Subcommand {
    /// Key management cli utilities
    #[command(subcommand)]
    Key(sc_cli::KeySubcommand),

    /// Build a chain specification.
    /// DEPRECATED: `build-spec` command will be removed after 1/04/2026. Use `export-chain-spec`
    /// command instead.
    #[deprecated(
        note = "build-spec command will be removed after 1/04/2026. Use export-chain-spec command instead"
    )]
    BuildSpec(sc_cli::BuildSpecCmd),

    /// Validate blocks.
    CheckBlock(sc_cli::CheckBlockCmd),

    /// Export blocks.
    ExportBlocks(sc_cli::ExportBlocksCmd),

    /// Export the state of a given block into a chain spec.
    ExportState(sc_cli::ExportStateCmd),

    /// Import blocks.
    ImportBlocks(sc_cli::ImportBlocksCmd),

    /// Remove the whole chain.
    PurgeChain(sc_cli::PurgeChainCmd),

    /// Revert the chain to a previous state.
    Revert(sc_cli::RevertCmd),

    /// Sub-commands concerned with benchmarking.
    #[command(subcommand)]
    Benchmark(frame_benchmarking_cli::BenchmarkCmd),

    /// Db meta columns information.
    ChainInfo(sc_cli::ChainInfoCmd),

    /// Ghost-specific commands.
    #[command(subcommand)]
    Ghost(GhostCommands),
}

/// Ghost-specific commands.
#[derive(Debug, clap::Subcommand)]
pub enum GhostCommands {
    /// Verify the PoW seal of a block in the local chain database.
    ///
    /// Recomputes the Ghost PoW check: pops the trailing
    /// `Seal(POW_ENGINE_ID, GhostSeal)` digest, re-hashes the header for the
    /// pre-hash, pulls the `PreRuntime(POW_ENGINE_ID, AccountId32)` miner
    /// digest, and evaluates `hash * difficulty <= U256::MAX` with the
    /// difficulty the runtime recorded for the parent.
    #[command(name = "verify-pow")]
    VerifyPow(VerifyPowCmd),
}

/// `ghost verify-pow <BLOCK_HASH>` arguments.
#[derive(Debug, clap::Args)]
pub struct VerifyPowCmd {
    /// Block hash (hex, `0x`-prefixed) of a block present in the local database.
    #[arg(value_name = "BLOCK_HASH")]
    pub block_hash: String,

    #[command(flatten)]
    pub shared_params: sc_cli::SharedParams,
}

impl sc_cli::CliConfiguration for VerifyPowCmd {
    fn shared_params(&self) -> &sc_cli::SharedParams {
        &self.shared_params
    }
}
