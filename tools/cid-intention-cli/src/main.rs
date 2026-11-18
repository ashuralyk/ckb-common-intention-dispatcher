//! Offline CKB-only CID Intention creator + explicit testnet send (no auto-broadcast).

use std::path::PathBuf;
use std::process::ExitCode;

use cid_intention_cli::{
    create_signed_intention, load_privkey_from_env, send_signed_transaction, CliError,
    CreateRequest, SendOptions, PRIVKEY_ENV,
};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "cid-intention-cli",
    about = "Offline CKB-only CID Intention create + env-key signing; explicit testnet send"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build and offline-sign an Intention create transaction (never broadcasts).
    Create {
        /// Path to offline request JSON (no secrets).
        #[arg(long)]
        request: PathBuf,
        /// Where to write the signed transaction JSON.
        #[arg(long)]
        output: PathBuf,
    },
    /// Broadcast a previously signed tx JSON to a TESTNET RPC (requires genesis guard).
    Send {
        /// Signed tx JSON from `create` (no private key required).
        #[arg(long)]
        tx: PathBuf,
        /// CKB JSON-RPC URL (use a local mock in CI/dev; do not hit public nets from agents).
        #[arg(long)]
        rpc_url: String,
        /// Must be `testnet`.
        #[arg(long, default_value = "testnet")]
        network: String,
        /// Expected genesis hash from `get_block_hash("0x0")`.
        #[arg(long)]
        genesis_hash: String,
    },
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Create { request, output } => {
            let text = std::fs::read_to_string(&request)?;
            let req: CreateRequest = serde_json::from_str(&text)?;
            let key = load_privkey_from_env()?;
            let tx = create_signed_intention(&req, &key)?;
            let _ = key;
            let out = serde_json::to_vec_pretty(&tx)?;
            std::fs::write(&output, out)?;
            eprintln!(
                "wrote signed intention tx to {} (not broadcast; use `send` explicitly; key via {PRIVKEY_ENV} only)",
                output.display()
            );
            Ok(())
        }
        Command::Send {
            tx,
            rpc_url,
            network,
            genesis_hash,
        } => {
            let text = std::fs::read_to_string(&tx)?;
            let signed: serde_json::Value = serde_json::from_str(&text)?;
            let opts = SendOptions {
                rpc_url,
                network,
                genesis_hash,
            };
            let hash = send_signed_transaction(&opts, &signed)?;
            println!("{hash}");
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cid-intention-cli: {}", e.safe_message());
            ExitCode::FAILURE
        }
    }
}
