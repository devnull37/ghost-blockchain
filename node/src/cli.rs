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

    /// Stake tokens for PoS validation (signs + submits to a running node)
    #[command(name = "stake")]
    Stake {
        /// Amount to stake in raw runtime balance units
        #[arg(long)]
        amount: u128,

        /// Signer secret URI / dev seed (e.g. //Alice). Defaults to //Alice.
        #[arg(long)]
        account: Option<String>,

        /// Node JSON-RPC endpoint. Defaults to http://127.0.0.1:9944.
        #[arg(long)]
        rpc_url: Option<String>,
    },

    /// Unstake tokens (signs + submits to a running node)
    #[command(name = "unstake")]
    Unstake {
        /// Amount to unstake in raw runtime balance units
        #[arg(long)]
        amount: u128,

        /// Signer secret URI / dev seed (e.g. //Alice). Defaults to //Alice.
        #[arg(long)]
        account: Option<String>,

        /// Node JSON-RPC endpoint. Defaults to http://127.0.0.1:9944.
        #[arg(long)]
        rpc_url: Option<String>,
    },

    /// Transfer balance to another account (signs + submits to a running node)
    #[command(name = "transfer")]
    Transfer {
        /// Destination: an SS58 address or a dev seed (e.g. //Bob)
        #[arg(long)]
        to: String,

        /// Amount to transfer in raw runtime balance units
        #[arg(long)]
        amount: u128,

        /// Signer secret URI / dev seed (e.g. //Alice). Defaults to //Alice.
        #[arg(long)]
        account: Option<String>,

        /// Node JSON-RPC endpoint. Defaults to http://127.0.0.1:9944.
        #[arg(long)]
        rpc_url: Option<String>,
    },

    /// Query a live account balance from a running node
    #[command(name = "balance")]
    Balance {
        /// Account: an SS58 address or a dev seed (e.g. //Alice). Defaults to //Alice.
        #[arg(long)]
        account: Option<String>,

        /// Node JSON-RPC endpoint. Defaults to http://127.0.0.1:9944.
        #[arg(long)]
        rpc_url: Option<String>,
    },

    /// Show live consensus and record-only PQ metadata status
    #[command(name = "status")]
    Status {
        /// Show detailed information
        #[arg(long)]
        detailed: bool,
    },

    /// List the live staked validator set from a running node
    #[command(name = "validators")]
    Validators {
        /// Reserved for future filtering; currently all staked validators are shown
        #[arg(long)]
        active_only: bool,

        /// Node JSON-RPC endpoint. Defaults to http://127.0.0.1:9944.
        #[arg(long)]
        rpc_url: Option<String>,
    },

    /// Generate an ML-DSA-87 (FIPS 204) keypair to <out>.pub and <out>.sec
    #[command(name = "pq-keygen")]
    PqKeygen {
        /// Output path prefix (writes <out>.pub and <out>.sec)
        #[arg(long, default_value = "ghost-mldsa")]
        out: String,
    },

    /// Register an ML-DSA-87 public key on-chain (signs + submits to a running node)
    #[command(name = "register-key")]
    RegisterKey {
        /// Path to the 2592-byte ML-DSA-87 public key file (from `pq-keygen`)
        #[arg(long)]
        key: String,

        /// Signer secret URI / dev seed (e.g. //Alice). Defaults to //Alice.
        #[arg(long)]
        account: Option<String>,

        /// Node JSON-RPC endpoint. Defaults to http://127.0.0.1:9944.
        #[arg(long)]
        rpc_url: Option<String>,
    },
}
