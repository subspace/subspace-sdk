use bytesize::ByteSize;
use futures::StreamExt;
use subspace_sdk::farmer::CacheDescription;
use subspace_sdk::node::NetworkBuilder;
use subspace_sdk::{chain_spec, node, Farmer, Node, PlotDescription, PublicKey};

#[tokio::main]
async fn main() {
    let node: Node = Node::builder()
        .blocks_pruning(node::BlocksPruning::Some(1000))
        .state_pruning(node::PruningMode::ArchiveCanonical)
        .network(NetworkBuilder::new().name("i1i1"))
        .build("node", chain_spec::dev_config().unwrap())
        .await
        .expect("Failed to init a node");

    node.sync().await.unwrap();

    let reward_address = PublicKey::from([0; 32]);
    let plots = [PlotDescription::new("plot", ByteSize::gb(10)).unwrap()];
    let farmer: Farmer = Farmer::builder()
        // .ws_rpc("127.0.0.1:9955".parse().unwrap())
        // .listen_on("/ip4/0.0.0.0/tcp/40333".parse().unwrap())
        .build(
            reward_address,
            node.clone(),
            &plots,
            CacheDescription::new("cache", ByteSize::mib(100)).unwrap(),
        )
        .await
        .expect("Failed to init a farmer");

    tokio::spawn({
        let mut solutions =
            farmer.iter_plots().await.next().unwrap().subscribe_new_solutions().await;
        async move {
            while let Some(solution) = solutions.next().await {
                eprintln!("Found solution: {solution:?}");
            }
        }
    });
    tokio::spawn({
        let mut new_blocks = node.subscribe_new_blocks().await.unwrap();
        async move {
            while let Some(block) = new_blocks.next().await {
                eprintln!("New block: {block:?}");
            }
        }
    });

    dbg!(node.get_info().await.unwrap());
    dbg!(farmer.get_info().await.unwrap());

    farmer.close().await.unwrap();
    node.close().await;

    // Restarting
    let node = Node::builder()
        .blocks_pruning(node::BlocksPruning::Some(1000))
        .state_pruning(node::PruningMode::ArchiveCanonical)
        .build("node", chain_spec::dev_config().unwrap())
        .await
        .expect("Failed to init a node");
    node.sync().await.unwrap();

    let farmer = Farmer::builder()
        .build(
            reward_address,
            node.clone(),
            &[PlotDescription::new("plot", ByteSize::gb(10)).unwrap()],
            CacheDescription::new("cache", ByteSize::mib(100)).unwrap(),
        )
        .await
        .expect("Failed to init a farmer");

    farmer.close().await.unwrap();
    node.close().await;

    // Delete everything
    for plot in plots {
        plot.wipe().await.unwrap();
    }
    Node::wipe("node").await.unwrap();
}
