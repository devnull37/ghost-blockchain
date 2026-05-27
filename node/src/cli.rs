#[derive(Debug, clap::Parser)]
#[command(
    name = "ghost-node",
    about = "Ghost node with classical transport, Proof-of-Work block authoring, longest-chain selection, and an experimental Ghost runtime pallet",
    long_about = "Ghost node with classical transport (libp2p/litep2p), Proof-of-Work block authoring, longest-chain (PoW) finality, and an experimental Ghost runtime pallet.\n\nOn-chain ML-DSA-87 signature verification is active in the Ghost consensus pallet. An ML-KEM-1024 encryption module exists node-side. libp2p transport remains classical.\n\nAny PQ fields exposed by this CLI are record-only metadata fields. They are non-enforcing runtime records and opaque attestation envelopes for claim tracking, not live post-quantum transport, authoring, finality, or quantum encryption.",
    version,
    author
)]
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

    /// Ghost blockchain specific commands
    #[command(subcommand)]
    Ghost(GhostCommands),
}

/// Ghost blockchain specific commands
#[derive(Debug, clap::Subcommand)]
pub enum GhostCommands {
    /// Run the local PoW demo miner
    #[command(name = "mine")]
    Mine {
        /// Number of CPU threads to use for the local demo miner
        #[arg(long, default_value = "1", value_parser = clap::value_parser!(u16).range(1..))]
        threads: u16,

        /// Conventional mining difficulty (numerically larger = harder, same convention
        /// as the runtime). Defaults to 1_000_000.
        #[arg(long)]
        difficulty: Option<u64>,
    },

    /// Stake tokens for PoS validation
    #[command(name = "stake")]
    Stake {
        /// Amount to stake in raw runtime balance units
        #[arg(long)]
        amount: u128,

        /// Account to stake from (if not provided, uses default account)
        #[arg(long)]
        account: Option<String>,
    },

    /// Unstake tokens
    #[command(name = "unstake")]
    Unstake {
        /// Amount to unstake in raw runtime balance units
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

    /// Show live consensus and record-only PQ metadata status
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
