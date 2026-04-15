#[derive(Debug, clap::Parser)]
#[command(name = "ghost-node", about = "Ghost blockchain node", version, author)]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,

    #[clap(flatten)]
    pub run: sc_cli::RunCmd,
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

    /// Ghost-specific informational commands
    #[command(subcommand)]
    Ghost(GhostCommands),
}

/// Ghost-specific informational commands.
#[derive(Debug, clap::Subcommand)]
pub enum GhostCommands {
    /// Run the local PoW demo miner
    #[command(name = "mine")]
    Mine {
        /// Number of threads to use for mining
        #[arg(long, default_value = "1")]
        threads: usize,

        /// Mining difficulty target
        #[arg(long)]
        difficulty: Option<u64>,
    },

    /// Show the staking call shape used by the pallet prototype
    #[command(name = "stake")]
    Stake {
        /// Amount to stake (in Ghost tokens)
        #[arg(long)]
        amount: u128,

        /// Account to stake from (if not provided, uses default account)
        #[arg(long)]
        account: Option<String>,
    },

    /// Show the unstaking call shape used by the pallet prototype
    #[command(name = "unstake")]
    Unstake {
        /// Amount to unstake
        #[arg(long)]
        amount: u128,

        /// Account to unstake from
        #[arg(long)]
        account: Option<String>,
    },

    /// Check balance and staking information
    #[command(name = "balance")]
    Balance {
        /// Account to check (if not provided, shows all accounts)
        #[arg(long)]
        account: Option<String>,
    },

    /// Show an honest consensus status summary
    #[command(name = "status")]
    Status {
        /// Show detailed information
        #[arg(long)]
        detailed: bool,
    },

    /// Show validator information
    #[command(name = "validators")]
    Validators {
        /// Show only active validators
        #[arg(long)]
        active_only: bool,
    },
}
