//! Subspace SDK for easy running of both Subspace node and farmer

#![deny(missing_docs)]
#![feature(type_changing_struct_update, concat_idents)]

/// Module related to the farmer
pub mod farmer;
pub(crate) mod networking;
/// Module related to the node
pub mod node;
pub(crate) mod utils;

use derive_more::{Deref, DerefMut};
pub use farmer::{Builder as FarmerBuilder, Farmer, Info as NodeInfo, Plot, PlotDescription};
pub use node::{chain_spec, Builder as NodeBuilder, Info as FarmerInfo, Node};
pub use parse_ss58::Ss58ParsingError;
use serde::{Deserialize, Serialize};
use subspace_core_primitives::PUBLIC_KEY_LENGTH;

#[doc(hidden)]
#[macro_export]
macro_rules! generate_builder {
    ( $name:ident ) => {
        impl concat_idents!($name, Builder) {
            /// Constructor
            pub fn new() -> Self {
                Self::default()
            }

            #[doc = concat!("Build ", stringify!($name))]
            pub fn build(&self) -> $name {
                self._build().expect("Infallible")
            }
        }

        impl From<concat_idents!($name, Builder)> for $name {
            fn from(value: concat_idents!($name, Builder)) -> Self {
                value.build()
            }
        }
    };
    ( $name:ident, $($rest:ident),+ ) => {
        $crate::generate_builder!($name);
        $crate::generate_builder!($($rest),+);
    };
}

/// Public key type
#[derive(
    Debug,
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    Hash,
    Deref,
    DerefMut,
    Serialize,
    Deserialize,
)]
#[serde(transparent)]
pub struct PublicKey(pub subspace_core_primitives::PublicKey);

impl PublicKey {
    /// Construct public key from raw bytes
    pub fn new(raw: [u8; PUBLIC_KEY_LENGTH]) -> Self {
        Self(subspace_core_primitives::PublicKey::from(raw))
    }
}

impl From<[u8; PUBLIC_KEY_LENGTH]> for PublicKey {
    fn from(key: [u8; PUBLIC_KEY_LENGTH]) -> Self {
        Self::new(key)
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

mod parse_ss58 {
    // Copyright (C) 2017-2022 Parity Technologies (UK) Ltd.
    // Copyright (C) 2022 Subspace Labs, Inc.
    // SPDX-License-Identifier: Apache-2.0

    // Licensed under the Apache License, Version 2.0 (the "License");
    // you may not use this file except in compliance with the License.
    // You may obtain a copy of the License at
    //
    // 	http://www.apache.org/licenses/LICENSE-2.0
    //
    // Unless required by applicable law or agreed to in writing, software
    // distributed under the License is distributed on an "AS IS" BASIS,
    // WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    // See the License for the specific language governing permissions and
    // limitations under the License.

    //! Modified version of SS58 parser extracted from Substrate in order to not
    //! pull the whole `sp-core` into farmer application

    use base58::FromBase58;
    use blake2::digest::typenum::U64;
    use blake2::digest::FixedOutput;
    use blake2::{Blake2b, Digest};
    use ss58_registry::Ss58AddressFormat;
    use subspace_core_primitives::{PublicKey, PUBLIC_KEY_LENGTH};
    use thiserror::Error;

    const PREFIX: &[u8] = b"SS58PRE";
    const CHECKSUM_LEN: usize = 2;

    /// An error type for SS58 decoding.
    #[derive(Debug, Error)]
    pub enum Ss58ParsingError {
        /// Base 58 requirement is violated
        #[error("Base 58 requirement is violated")]
        BadBase58,
        /// Length is bad
        #[error("Length is bad")]
        BadLength,
        /// Invalid SS58 prefix byte
        #[error("Invalid SS58 prefix byte")]
        InvalidPrefix,
        /// Disallowed SS58 Address Format for this datatype
        #[error("Disallowed SS58 Address Format for this datatype")]
        FormatNotAllowed,
        /// Invalid checksum
        #[error("Invalid checksum")]
        InvalidChecksum,
    }

    /// Some if the string is a properly encoded SS58Check address.
    pub(crate) fn parse_ss58_reward_address(s: &str) -> Result<PublicKey, Ss58ParsingError> {
        let data = s.from_base58().map_err(|_| Ss58ParsingError::BadBase58)?;
        if data.len() < 2 {
            return Err(Ss58ParsingError::BadLength);
        }
        let (prefix_len, ident) = match data[0] {
            0..=63 => (1, data[0] as u16),
            64..=127 => {
                // weird bit manipulation owing to the combination of LE encoding and missing
                // two bits from the left.
                // d[0] d[1] are: 01aaaaaa bbcccccc
                // they make the LE-encoded 16-bit value: aaaaaabb 00cccccc
                // so the lower byte is formed of aaaaaabb and the higher byte is 00cccccc
                let lower = (data[0] << 2) | (data[1] >> 6);
                let upper = data[1] & 0b00111111;
                (2, (lower as u16) | ((upper as u16) << 8))
            }
            _ => return Err(Ss58ParsingError::InvalidPrefix),
        };
        if data.len() != prefix_len + PUBLIC_KEY_LENGTH + CHECKSUM_LEN {
            return Err(Ss58ParsingError::BadLength);
        }
        let format: Ss58AddressFormat = ident.into();
        if format.is_reserved() {
            return Err(Ss58ParsingError::FormatNotAllowed);
        }

        let hash = ss58hash(&data[0..PUBLIC_KEY_LENGTH + prefix_len]);
        let checksum = &hash[0..CHECKSUM_LEN];
        if data[PUBLIC_KEY_LENGTH + prefix_len..PUBLIC_KEY_LENGTH + prefix_len + CHECKSUM_LEN]
            != *checksum
        {
            // Invalid checksum.
            return Err(Ss58ParsingError::InvalidChecksum);
        }

        let bytes: [u8; PUBLIC_KEY_LENGTH] = data[prefix_len..][..PUBLIC_KEY_LENGTH]
            .try_into()
            .map_err(|_| Ss58ParsingError::BadLength)?;

        Ok(PublicKey::from(bytes))
    }

    fn ss58hash(data: &[u8]) -> [u8; 64] {
        let mut state = Blake2b::<U64>::new();
        state.update(PREFIX);
        state.update(data);
        state.finalize_fixed().into()
    }

    /// ```
    /// "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY".parse::<subspace_sdk::PublicKey>().unwrap();
    /// ```
    impl std::str::FromStr for super::PublicKey {
        type Err = Ss58ParsingError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            parse_ss58_reward_address(s).map(Self)
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use subspace_farmer::node_client::NodeClient;
    use tempfile::TempDir;

    use super::farmer::CacheDescription;
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_integration() {
        let dir = TempDir::new().unwrap();
        let node = Node::builder()
            .force_authoring(true)
            .role(node::Role::Authority)
            .build(dir, node::chain_spec::dev_config().unwrap())
            .await
            .unwrap();

        let mut slot_info_sub = node.subscribe_slot_info().await.unwrap();

        let (dir, cache_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let plot_descriptions =
            [PlotDescription::new(dir.path(), PlotDescription::MIN_SIZE).unwrap()];
        let _farmer = Farmer::builder()
            .build(
                Default::default(),
                node.clone(),
                &plot_descriptions,
                CacheDescription::new(cache_dir.as_ref(), CacheDescription::MIN_SIZE).unwrap(),
            )
            .await
            .unwrap();

        // New slots arrive at each block. So basically we wait for 3 blocks to produce
        for _ in 0..3 {
            assert!(slot_info_sub.next().await.is_some());
        }
    }

    #[test]
    fn check_toml_serializable_max() {
        const DEFAULT_NODE_CONFIGS: &[&str] = &[
            r#"
piece_cache_size = "1073.7 MB"
segment_publish_concurrency = 10
force_authoring = false
role = "Full"
blocks_pruning = "KeepAll"
state_pruning = "ArchiveAll"
execution_strategy = "NativeWhenPossible"
impl_name = "subspace-sdk"
impl_version = "0.1.0-c88c59ab7a7cae3518b224302a3c96beb7c5afaf"

[rpc]
methods = "Auto"
max_subs_per_conn = 1024

[network]
enable_mdns = false
allow_private_ipv4 = false
listen_addresses = []
boot_nodes = []
force_synced = false

[offchain_worker]
enabled = false
indexing_enabled = false

[system_domain]
relayer_id = "5C4hrfjw9DjXZTzV3MwzrrAr9P1MJhSrvWGWqi1eSuyUpnhM"
force_authoring = false
role = "Full"
blocks_pruning = "KeepAll"
state_pruning = "ArchiveAll"
execution_strategy = "NativeWhenPossible"
impl_name = "subspace-sdk"
impl_version = "0.1.0-c88c59ab7a7cae3518b224302a3c96beb7c5afaf"

[system_domain.rpc]
methods = "Auto"
max_subs_per_conn = 1024

[system_domain.network]
enable_mdns = false
allow_private_ipv4 = false
listen_addresses = []
boot_nodes = []
force_synced = false

[system_domain.offchain_worker]
enabled = false
indexing_enabled = false

[system_domain.core]
relayer_id = "5C4hrfjw9DjXZTzV3MwzrrAr9P1MJhSrvWGWqi1eSuyUpnhM"
force_authoring = false
role = "Full"
blocks_pruning = "KeepAll"
state_pruning = "ArchiveAll"
execution_strategy = "NativeWhenPossible"
impl_name = "subspace-sdk"
impl_version = "0.1.0-c88c59ab7a7cae3518b224302a3c96beb7c5afaf"

[system_domain.core.rpc]
methods = "Auto"
max_subs_per_conn = 1024

[system_domain.core.network]
enable_mdns = false
allow_private_ipv4 = false
listen_addresses = []
boot_nodes = []
force_synced = false

[system_domain.core.offchain_worker]
enabled = false
indexing_enabled = false
"#,
            r#""#,
        ];
        const DEFAULT_FARMER_CONFIGS: &[&str] = &[
            r#"
[dsn]
listen_addresses = ["/ip4/127.0.0.1/tcp/0"]
boot_nodes = []
reserved_nodes = []
allow_non_global_addresses_in_dht = false
piece_publisher_batch_size = 15

piece_receiver_batch_size = 12
piece_publisher_batch_size = 12
max_concurrent_plots = 10
"#,
            r#"
piece_receiver_batch_size = 12
piece_publisher_batch_size = 12
max_concurrent_plots = 10
"#,
            r#""#,
        ];

        for node in DEFAULT_NODE_CONFIGS {
            let node = toml::from_str::<node::Config>(node).unwrap();
            toml::to_string(&node).unwrap();
        }
        for farmer in DEFAULT_FARMER_CONFIGS {
            let farmer = toml::from_str::<farmer::Config>(farmer).unwrap();
            toml::to_string(&farmer).unwrap();
        }
    }
}
