use std::path::Path;
use std::sync::{Arc, Weak};

use anyhow::Context;
use core_payments_domain_runtime::RelayerId;
use derivative::Derivative;
use derive_builder::Builder;
use domain_service::DomainConfiguration;
use futures::prelude::*;
use sc_client_api::client::BlockImportNotification;
use serde::{Deserialize, Serialize};
use sp_domains::DomainId;
use subspace_runtime_primitives::opaque::Header;

use crate::node::{Base, BaseBuilder, BlockNotification};

/// Core payments domain executor instance.
pub struct ExecutorDispatch;

impl sc_executor::NativeExecutionDispatch for ExecutorDispatch {
    // #[cfg(feature = "runtime-benchmarks")]
    // type ExtendHostFunctions = frame_benchmarking::benchmarking::HostFunctions;
    // #[cfg(not(feature = "runtime-benchmarks"))]
    type ExtendHostFunctions = ();

    fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
        core_payments_domain_runtime::api::dispatch(method, data)
    }

    fn native_version() -> sc_executor::NativeVersion {
        core_payments_domain_runtime::native_version()
    }
}

/// Node builder
#[derive(Clone, Derivative, Builder, Deserialize, Serialize)]
#[derivative(Debug)]
#[builder(pattern = "immutable", build_fn(private, name = "_build"), name = "ConfigBuilder")]
#[non_exhaustive]
pub struct Config {
    #[doc(hidden)]
    #[builder(
        setter(into, strip_option),
        field(type = "BaseBuilder", build = "self.base.build()")
    )]
    #[serde(default)]
    pub base: Base,
    #[derivative(Debug = "ignore")]
    #[builder(setter(skip), field(type = "()", build = "None"))]
    chain_spec: Option<ChainSpec>,
    /// Id of the relayer
    #[builder(setter(strip_option), default)]
    #[serde(default)]
    pub relayer_id: Option<RelayerId>,
}

impl ConfigBuilder {
    /// Constructor
    pub fn new() -> Self {
        Self::default()
    }

    /// Build Config
    pub fn build(&self, chain_spec: ChainSpec) -> Config {
        Config { chain_spec: Some(chain_spec), ..self._build().expect("Infallible") }
    }
}

pub(crate) type FullClient =
    domain_service::FullClient<core_payments_domain_runtime::RuntimeApi, ExecutorDispatch>;
pub(crate) type NewFull = domain_service::NewFullCore<
    Arc<FullClient>,
    sc_executor::NativeElseWasmExecutor<ExecutorDispatch>,
    sp_runtime::generic::Block<
        sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>,
        sp_runtime::OpaqueExtrinsic,
    >,
    subspace_runtime_primitives::opaque::Block,
    super::FullClient,
    crate::node::PrimaryFullClient,
    core_payments_domain_runtime::RuntimeApi,
    ExecutorDispatch,
>;
pub(crate) type ChainSpec =
    sc_subspace_chain_specs::ExecutionChainSpec<core_payments_domain_runtime::GenesisConfig>;

/// Secondary executor node
#[derive(Clone)]
pub struct CoreNode {
    client: Weak<FullClient>,
    rpc_handlers: crate::utils::Rpc,
}

impl CoreNode {
    pub(crate) async fn new(
        cfg: Config,
        directory: impl AsRef<Path>,
        primary_chain_node: &mut crate::node::PrimaryNewFull,
        secondary_node: &super::NewFull,
        gossip_msg_sink: domain_client_message_relayer::GossipMessageSink,
        domain_tx_pool_sinks: &mut impl Extend<(
            DomainId,
            cross_domain_message_gossip::DomainTxPoolSink,
        )>,
    ) -> anyhow::Result<Self> {
        let Config { base, relayer_id: maybe_relayer_id, chain_spec } = cfg;
        let chain_spec = chain_spec.expect("Always set in builder");
        let service_config = base.configuration(directory, chain_spec).await;
        let core_domain_config = DomainConfiguration { service_config, maybe_relayer_id };

        // TODO: proper value
        let block_import_throttling_buffer_size = 10;
        let imported_block_notification_stream = primary_chain_node
            .imported_block_notification_stream
            .subscribe()
            .then(|imported_block_notification| async move {
                (
                    imported_block_notification.block_number,
                    imported_block_notification.fork_choice,
                    imported_block_notification.block_import_acknowledgement_sender,
                )
            });
        let new_slot_notification_stream = primary_chain_node
            .new_slot_notification_stream
            .subscribe()
            .then(|slot_notification| async move {
                (
                    slot_notification.new_slot_info.slot,
                    slot_notification.new_slot_info.global_challenge,
                )
            });

        let NewFull { client, rpc_handlers, tx_pool_sink, task_manager, network_starter, .. } =
            domain_service::new_full_core(
                DomainId::CORE_PAYMENTS,
                core_domain_config,
                secondary_node.client.clone(),
                secondary_node.network.clone(),
                primary_chain_node.client.clone(),
                primary_chain_node.network.clone(),
                &primary_chain_node.select_chain,
                imported_block_notification_stream,
                new_slot_notification_stream,
                block_import_throttling_buffer_size,
                gossip_msg_sink,
            )
            .await?;

        domain_tx_pool_sinks.extend([(DomainId::CORE_PAYMENTS, tx_pool_sink)]);
        primary_chain_node.task_manager.add_child(task_manager);

        network_starter.start_network();

        Ok(Self {
            client: Arc::downgrade(&client),
            rpc_handlers: crate::utils::Rpc::new(&rpc_handlers),
        })
    }

    pub(crate) fn client(&self) -> anyhow::Result<Arc<FullClient>> {
        self.client.upgrade().ok_or_else(|| anyhow::anyhow!("The node was already closed"))
    }

    /// Subscribe to new blocks imported
    pub async fn subscribe_new_blocks(
        &self,
    ) -> anyhow::Result<impl Stream<Item = BlockNotification> + Send + Sync + Unpin + 'static> {
        use sc_client_api::client::BlockchainEvents;

        let stream = self
            .client()
            .context("Failed to subscribe to new blocks")?
            .import_notification_stream()
            .map(
                |BlockImportNotification {
                     hash,
                     header: Header { parent_hash, number, state_root, extrinsics_root, digest: _ },
                     origin: _,
                     is_new_best,
                     tree_route: _,
                 }| BlockNotification {
                    hash,
                    number,
                    parent_hash,
                    state_root,
                    extrinsics_root,
                    is_new_best,
                },
            );
        Ok(stream)
    }
}
