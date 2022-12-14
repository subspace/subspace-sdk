use std::path::PathBuf;

use anyhow::Context;
use bytesize::ByteSize;
use clap::Parser;
use futures::prelude::*;
use subspace_sdk::farmer::CacheDescription;
use subspace_sdk::node::{self, Node};
use subspace_sdk::{Farmer, PlotDescription, PublicKey};

/// Gemini 3b test binary
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Args {
    /// Address for farming rewards
    #[arg(short, long)]
    reward_address: PublicKey,
    /// Path for all data
    #[arg(short, long)]
    base_path: Option<PathBuf>,
    /// Size of the plot
    #[arg(short, long)]
    plot_size: ByteSize,
    /// Cache size
    #[arg(short, long, default_value_t = ByteSize::gib(1))]
    cache_size: ByteSize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap()),
        )
        .init();

    let Args { reward_address, base_path, plot_size, cache_size } = Args::parse();
    let (base_path, _tmp_dir) = base_path.map(|x| (x, None)).unwrap_or_else(|| {
        let tmp = tempfile::tempdir().expect("Failed to create temporary directory");
        (tmp.as_ref().to_owned(), Some(tmp))
    });

    let node_dir = base_path.join("node");
    let node = Node::builder()
        .role(node::Role::Authority)
        .network(
            node::NetworkBuilder::new()
                .listen_addresses(vec![
                    "/ip6/::/tcp/30333".parse().unwrap(),
                    "/ip4/0.0.0.0/tcp/30333".parse().unwrap(),
                ])
                .enable_mdns(true),
        )
        .rpc(
            node::RpcBuilder::new()
                .http("127.0.0.1:9933".parse().unwrap())
                .ws("127.0.0.1:9944".parse().unwrap())
                .cors(vec![
                    "http://localhost:*".to_owned(),
                    "http://127.0.0.1:*".to_owned(),
                    "https://localhost:*".to_owned(),
                    "https://127.0.0.1:*".to_owned(),
                    "https://polkadot.js.org".to_owned(),
                ]),
        )
        .dsn(node::DsnBuilder::new().listen_addresses(vec![
            "/ip6/::/tcp/30433".parse().unwrap(),
            "/ip4/0.0.0.0/tcp/30433".parse().unwrap(),
        ]))
        .execution_strategy(node::ExecutionStrategy::AlwaysWasm)
        .offchain_worker(node::OffchainWorkerBuilder::new().enabled(true))
        .build(&node_dir, node::chain_spec::gemini_3b().unwrap())
        .await?;

    tokio::select! {
        result = node.sync() => result?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Exitting...");
            return Ok(())
        }
    }
    tracing::info!("Node was synced!");

    let farmer = Farmer::builder()
        .build(
            reward_address,
            node.clone(),
            &[PlotDescription::new(base_path.join("plot"), plot_size)
                .context("Failed to create a plot")?],
            CacheDescription::new(base_path.join("cache"), cache_size).unwrap(),
        )
        .await?;
    let plot = farmer.iter_plots().await.next().unwrap();

    let subscriptions = async move {
        plot.subscribe_initial_plotting_progress()
            .await
            .for_each(|progress| async move {
                tracing::info!(?progress, "Plotting!");
            })
            .await;
        plot.subscribe_new_solutions()
            .await
            .for_each(|_| async move {
                tracing::info!("Farmed another solution!");
            })
            .await;
    };

    tokio::select! {
        _ = subscriptions => {},
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Exitting...");
            return Ok(())
        }
    }

    node.close().await;
    farmer.close().await.context("Failed to close farmer")?;

    Ok(())
}
