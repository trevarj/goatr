// SPDX-License-Identifier: GPL-3.0-or-later
//! Retained identity, private local administration and authenticated direct iroh runtime.

use std::time::Duration;

use iroh::{
    Endpoint, SecretKey,
    endpoint::{Builder, PortmapperConfig, QuicTransportConfig, presets},
};

pub mod admin;
pub mod attachments;
pub mod backends;
pub mod codex_relay;
pub mod config;
pub mod herdr;
pub mod native_json;
pub mod pairing;
#[cfg(target_os = "linux")]
mod project_read;
pub mod protocol;
pub mod push;
pub mod setup;
pub mod state;
pub mod storage;
pub mod transport;
mod hc_bridge;

pub const ALPN: &[u8] = b"goatr/2";
/// Pair/hello JSON frames, then an authorized hc-connect byte stream on the same stream.
pub const HC_ALPN: &[u8] = b"goatr/hc/1";
pub(crate) fn authorized_alpn(alpn: &[u8]) -> bool {
    alpn == ALPN || alpn == HC_ALPN
}

/// The caller supplies its installation's retained secret, never a shared key.
pub fn endpoint_builder(secret: SecretKey) -> Builder {
    Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .portmapper_config(PortmapperConfig::Disabled)
        .transport_config(
            QuicTransportConfig::builder()
                .max_concurrent_bidi_streams(1u32.into())
                .max_concurrent_uni_streams(0u32.into())
                .build(),
        )
}

/// Close connections and native tasks before replacing an endpoint.
pub async fn close_endpoint(endpoint: &Endpoint) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(5), endpoint.close()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recreation_keeps_identity_and_closes_endpoint() {
        let secret = SecretKey::from_bytes(&[7; 32]);
        let endpoint = endpoint_builder(secret.clone())
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        assert_eq!(endpoint.id(), secret.public());
        endpoint.network_change().await;
        close_endpoint(&endpoint).await.unwrap();
        assert!(endpoint.is_closed());

        let recreated = endpoint_builder(secret)
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        assert_eq!(endpoint.id(), recreated.id());
        close_endpoint(&recreated).await.unwrap();
    }
}
