// SPDX-License-Identifier: GPL-3.0-or-later
//! Durable notification hints only. No provider instructions or endpoint logs.
use crate::{protocol::*, storage::AuthenticatedPeer};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, SystemTime},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    pub peer: Uuid,
    pub epoch: u64,
    pub generation: Uuid,
    pub endpoint: String,
    pub key: Key32,
    pub probe: Uuid,
    pub registered: u64,
    pub deadline: u64,
    pub verified: bool,
    pub disabled: bool,
}
impl std::fmt::Debug for Registration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PushRegistration(redacted)")
    }
}
#[derive(Clone)]
pub(crate) struct Attempt {
    pub proof: AuthenticatedPeer,
    pub registration: Registration,
    pub id: Uuid,
    pub event: Option<AttentionEvent>,
    pub attempts: u64,
    pub expires: u64,
}
impl std::fmt::Debug for Attempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PushAttempt(redacted)")
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Publication {
    Accepted,
    Retry { after: Option<u64> },
    Rejected,
    Disabled,
    Canceled,
}

/// Retain the caller's full URL, but authorize only an exact desktop-local origin.
pub(crate) fn endpoint(value: &str, allowed: &[String]) -> Result<url::Url, ErrorCode> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(ErrorCode::PushRejected);
    }
    let url = url::Url::parse(value).map_err(|_| ErrorCode::PushRejected)?;
    let raw_authority = value
        .split_once("://")
        .map(|(_, tail)| tail.split(['/', '?', '#']).next().unwrap_or(""))
        .ok_or(ErrorCode::PushRejected)?;
    if raw_authority.contains('@') {
        return Err(ErrorCode::PushRejected);
    }
    let suffix =
        &value[value.find("://").ok_or(ErrorCode::PushRejected)? + 3 + raw_authority.len()..];
    let expected = if suffix.is_empty() {
        "/".to_owned()
    } else if suffix.starts_with('?') {
        format!("/{suffix}")
    } else {
        suffix.to_owned()
    };
    let actual = format!(
        "{}{}",
        url.path(),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    if expected != actual {
        return Err(ErrorCode::PushRejected);
    }
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ErrorCode::PushRejected);
    }
    let origin = url.origin().ascii_serialization();
    if !allowed.iter().any(|s| s == &origin) {
        return Err(ErrorCode::PushRejected);
    }
    Ok(url)
}
fn permitted(ip: IpAddr, private: bool) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, d] = v.octets();
            if a == 0
                || a == 127
                || a >= 224
                || (a == 169 && b == 254)
                || (a == 100 && b == 100 && c == 100 && d == 200)
                || (a == 168 && b == 63 && c == 129 && d == 16)
            {
                return false;
            }
            if private {
                return true;
            }
            !(a == 10
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || (a == 192 && b == 0)
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return permitted(IpAddr::V4(v4), private);
            }
            let s = v.segments();
            if v.is_unspecified()
                || v.is_loopback()
                || v.is_multicast()
                || (s[0] & 0xffc0) == 0xfe80
                || ((s[0] == 0xfd00 && s[1] == 0xec2 || s[0] == 0xfd20 && s[1] == 0xce)
                    && s[7] == 0x254)
            {
                return false;
            }
            if private {
                return true;
            }
            (s[0] & 0xe000) == 0x2000 && !(s[0] == 0x2001 && s[1] == 0xdb8)
        }
    }
}
impl Attempt {
    pub(crate) fn hint(&self, host: &EndpointId) -> Result<Vec<u8>, ErrorCode> {
        let r = &self.registration;
        let payload = if let Some(event) = &self.event {
            serde_json::json!({"v":1,"host":host,"device":r.peer,"endpointGeneration":r.generation,"sessionId":event.session_id,"eventId":event.event_id,"sequence":event.sequence,"category":event.category})
        } else {
            serde_json::json!({"v":1,"host":host,"device":r.peer,"endpointGeneration":r.generation,"probe":r.probe,"category":"probe"})
        };
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).map_err(|_| ErrorCode::NotReady)?);
        let mut key = URL_SAFE_NO_PAD
            .decode(&r.key.0)
            .map_err(|_| ErrorCode::NotReady)?;
        if key.len() != 32 {
            return Err(ErrorCode::NotReady);
        }
        let prefix = format!("G1.{encoded}");
        let mac = URL_SAFE_NO_PAD.encode(hmac_sha256(&key, prefix.as_bytes()));
        key.fill(0);
        let hint = format!("{prefix}.{mac}").into_bytes();
        if hint.len() > 2048 {
            return Err(ErrorCode::ResourceLimit);
        }
        Ok(hint)
    }
}
/// A fresh, proxy-free, nonretrying client with a single validated DNS result set.
/// The outer deadline includes resolution; no response bodies/endpoints are logged.
pub(crate) async fn publish(
    url: url::Url,
    hint: Vec<u8>,
    mut cancel: tokio::sync::watch::Receiver<u64>,
    security: &crate::pairing::Security,
    attempt: &Attempt,
) -> Publication {
    let observed_cancel = cancel.clone();
    let operation = async move {
        let host = url
            .host_str()
            .ok_or(())?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = url.port_or_known_default().ok_or(())?;
        let private = url.origin().ascii_serialization() != "https://ntfy.sh";
        let addresses: Vec<SocketAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, port)],
            Err(_) => {
                let mut addresses = Vec::new();
                for address in tokio::net::lookup_host((host.as_str(), port))
                    .await
                    .map_err(|_| ())?
                {
                    if addresses.len() >= 32 {
                        return Err(());
                    }
                    addresses.push(address)
                }
                addresses
            }
        };
        if addresses.is_empty() || addresses.iter().any(|a| !permitted(a.ip(), private)) {
            return Ok(Publication::Rejected);
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .resolve_to_addrs(&host, &addresses)
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| ())?;
        // DNS may have waited. Reauthorize the exact attempt after resolution and
        // before sending; cancellation also races any wait for this original gate.
        let guard = security.push_guard(&attempt.proof).await.map_err(|_| ())?;
        let check = attempt.clone();
        let admitted = security
            .storage
            .call(move |s| s.push_attempt_current(&check, crate::pairing::now_ms()))
            .await
            .map_err(|_| ())?;
        if !admitted || *observed_cancel.borrow() != 0 {
            return Ok(Publication::Canceled);
        }
        drop(guard);
        let response = client
            .post(url)
            .header("Content-Type", "application/octet-stream")
            .header("X-UnifiedPush", "1")
            .body(hint)
            .send()
            .await
            .map_err(|_| ())?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(Publication::Accepted);
        }
        if status == 404 || status == 410 {
            return Ok(Publication::Disabled);
        }
        if status == 429 || status >= 500 {
            let after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.parse::<u64>().ok().or_else(|| {
                        httpdate::parse_http_date(v).ok().map(|t| {
                            t.duration_since(SystemTime::now())
                                .unwrap_or_default()
                                .as_secs()
                        })
                    })
                });
            return Ok(Publication::Retry { after });
        }
        Ok(Publication::Rejected)
    };
    tokio::select! {biased;_=cancel.changed()=>Publication::Canceled,result=tokio::time::timeout(Duration::from_secs(10),operation)=>match result{Ok(Ok(result))=>result,_=>Publication::Retry{after:None}}}
}

struct ActiveAttempt {
    cancel: tokio::sync::watch::Sender<u64>,
    done: tokio::sync::watch::Receiver<bool>,
}
pub(crate) struct Publisher {
    allowed: Vec<String>,
    active: std::sync::Mutex<std::collections::HashMap<Uuid, ActiveAttempt>>,
}
impl Publisher {
    pub(crate) fn new(allowed: Vec<String>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            allowed,
            active: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }
    pub(crate) fn validate(&self, value: &str) -> Result<url::Url, ErrorCode> {
        endpoint(value, &self.allowed)
    }
    pub(crate) fn cancel(&self, peer: &Uuid) {
        if let Some(active) = self.active.lock().expect("push attempt mutex").get(peer) {
            active.cancel.send_modify(|n| *n = n.wrapping_add(1));
        }
    }
    pub(crate) fn cancel_all(&self) {
        for active in self.active.lock().expect("push attempt mutex").values() {
            active.cancel.send_modify(|n| *n = n.wrapping_add(1));
        }
    }
    /// A stopped task reports completion only after its network future has ended
    /// and been dropped. Durable result bookkeeping cannot block a caller's gate.
    pub(crate) async fn drain_peer(&self, peer: &Uuid) -> Result<(), ErrorCode> {
        let done = self
            .active
            .lock()
            .expect("push attempt mutex")
            .get(peer)
            .map(|active| {
                active.cancel.send_modify(|n| *n = n.wrapping_add(1));
                active.done.clone()
            });
        let Some(mut done) = done else { return Ok(()) };
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if *done.borrow() {
                    return Ok(());
                }
                done.changed().await.map_err(|_| ErrorCode::NotReady)?;
            }
        })
        .await
        .map_err(|_| ErrorCode::NotReady)?
    }
    fn failed(security: &crate::pairing::Security) {
        if !security
            .push_failed
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            eprintln!("push: durable_queue_unavailable");
        }
    }
    async fn attempt(
        self: std::sync::Arc<Self>,
        security: std::sync::Arc<crate::pairing::Security>,
        attempt: Attempt,
        receiver: tokio::sync::watch::Receiver<u64>,
        done: tokio::sync::watch::Sender<bool>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        let peer = attempt.proof.peer_id().clone();
        let mut canceled = receiver.clone();
        let operation = async {
            if *receiver.borrow() != 0 {
                return Ok::<_, ErrorCode>(Publication::Canceled);
            }
            let guard = security.push_guard(&attempt.proof).await?;
            let check = attempt.clone();
            let admitted = security
                .storage
                .call(move |s| s.push_attempt_current(&check, crate::pairing::now_ms()))
                .await
                .map_err(crate::pairing::store_error)?;
            let url = self.validate(&attempt.registration.endpoint);
            let hint = attempt.hint(&security.host);
            drop(guard);
            if !admitted {
                return Ok(Publication::Canceled);
            }
            Ok(match (url, hint) {
                (Ok(url), Ok(hint)) => {
                    tokio::select! {biased;_=stop.changed()=>Publication::Canceled,result=publish(url,hint,receiver,&security,&attempt)=>result}
                }
                _ => Publication::Rejected,
            })
        };
        let result = tokio::select! {biased;_=canceled.changed()=>Publication::Canceled,result=operation=>result.unwrap_or(Publication::Canceled)};
        done.send_replace(true);
        if security
            .storage
            .call(move |s| s.push_finish(attempt, result, crate::pairing::now_ms()))
            .await
            .is_err()
        {
            Self::failed(&security)
        }
        self.active
            .lock()
            .expect("push attempt mutex")
            .remove(&peer);
    }
    pub(crate) async fn run(
        self: std::sync::Arc<Self>,
        security: std::sync::Weak<crate::pairing::Security>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            tokio::select! {biased;_=stop.changed()=>break,_=jobs.join_next(),if !jobs.is_empty()=>{},_=tick.tick()=>{}}
            let Some(security) = security.upgrade() else {
                break;
            };
            while jobs.len() < 8 && self.active.lock().expect("push attempt mutex").len() < 8 {
                let excluded: Vec<_> = self
                    .active
                    .lock()
                    .expect("push attempt mutex")
                    .keys()
                    .cloned()
                    .collect();
                let attempt = match security
                    .storage
                    .call(move |s| s.push_next(crate::pairing::now_ms(), &excluded))
                    .await
                {
                    Ok(Some(a)) => a,
                    Ok(None) => break,
                    Err(_) => {
                        Self::failed(&security);
                        break;
                    }
                };
                let (sender, receiver) = tokio::sync::watch::channel(0u64);
                let (done, completion) = tokio::sync::watch::channel(false);
                self.active.lock().expect("push attempt mutex").insert(
                    attempt.proof.peer_id().clone(),
                    ActiveAttempt {
                        cancel: sender,
                        done: completion,
                    },
                );
                jobs.spawn(self.clone().attempt(
                    security.clone(),
                    attempt,
                    receiver,
                    done,
                    stop.clone(),
                ));
            }
        }
        self.cancel_all();
        if tokio::time::timeout(Duration::from_secs(10), async {
            while jobs.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            jobs.abort_all();
            while jobs.join_next().await.is_some() {}
        }
        self.active.lock().expect("push attempt mutex").clear();
    }
}

/// A native correlation is bounded and payload-free; it never hashes prompt,
/// tool, message or frame contents. Generation prevents cross-incarnation reuse.
pub(crate) fn correlation(target: &Target, provider: &str, native: &str, family: &str) -> Id {
    let bytes = serde_json::to_vec(&(target, provider, native, family))
        .expect("correlation tuple serialization");
    Id(format!("push:{}", crate::pairing::hex(&sha256(&bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_origin_consent_drives_the_real_endpoint_authorizer_without_empty_fallback() {
        let base = "version=1\nruntime_dir='/configured/runtime'\nruntime_dir_agent='/configured/runtime'\n[executables]\nherdr='/configured/herdr'\n";
        let omitted = crate::config::Config::parse(base).unwrap();
        let public = Publisher::new(omitted.push_allowed_origins);
        assert!(public.validate("https://ntfy.sh/topic?up=1").is_ok());
        assert_eq!(
            public.validate("https://ntfy.sh:8443/topic").unwrap_err(),
            ErrorCode::PushRejected
        );
        let deny =
            crate::config::Config::parse(&format!("push_allowed_origins=[]\n{base}")).unwrap();
        let deny = crate::config::Config::parse(&deny.serialized().unwrap()).unwrap();
        assert_eq!(
            Publisher::new(deny.push_allowed_origins)
                .validate("https://ntfy.sh/topic")
                .unwrap_err(),
            ErrorCode::PushRejected
        );
        let private = crate::config::Config::parse(&format!(
            "push_allowed_origins=['https://push.example:8443']\n{base}"
        ))
        .unwrap();
        let private = crate::config::Config::parse(&private.serialized().unwrap()).unwrap();
        let private = Publisher::new(private.push_allowed_origins);
        assert!(
            private
                .validate("https://push.example:8443/topic?exact=kept")
                .is_ok()
        );
        for refused in [
            "https://ntfy.sh/topic",
            "https://push.example/topic",
            "https://push.example:8443/a/../topic",
            "https://user@push.example:8443/topic",
            "http://push.example:8443/topic",
            "https://push.example:8443/topic#fragment",
        ] {
            assert_eq!(
                private.validate(refused).unwrap_err(),
                ErrorCode::PushRejected
            );
        }
    }

    #[test]
    fn actual_address_policy_keeps_public_private_and_metadata_boundaries_separate() {
        for value in [
            "127.0.0.1",
            "169.254.169.254",
            "100.100.100.200",
            "168.63.129.16",
            "::1",
            "fe80::1",
            "fd00:ec2::254",
        ] {
            let address: IpAddr = value.parse().unwrap();
            assert!(!permitted(address, false));
            assert!(!permitted(address, true));
        }
        for value in ["10.0.0.4", "192.168.0.7", "fd12::42"] {
            let address: IpAddr = value.parse().unwrap();
            assert!(!permitted(address, false));
            assert!(permitted(address, true));
        }
        assert!(permitted("8.8.8.8".parse().unwrap(), false));
    }
}
