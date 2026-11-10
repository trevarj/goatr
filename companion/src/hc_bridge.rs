// SPDX-License-Identifier: GPL-3.0-or-later
//! Only the local operator chooses the executable; peer bytes are stdin, never argv.
use crate::pairing::{Authorized, Security};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::{io, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn executable() -> io::Result<PathBuf> {
    let path = PathBuf::from(std::env::var_os("GOATR_HC").ok_or_else(|| {
        io::Error::other("GOATR_HC must select an absolute compatible hc executable")
    })?);
    if !path.is_absolute() || !path.is_file() {
        return Err(io::Error::other(
            "GOATR_HC must select an absolute compatible hc executable",
        ));
    }
    Ok(path)
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "bridge authorization ended",
    )
}

pub(crate) async fn run(
    connection: &Connection,
    security: &Arc<Security>,
    auth: &Authorized,
    mut send: SendStream,
    mut recv: RecvStream,
) -> io::Result<()> {
    let mut child = {
        let _guard = security.guard(auth).await.map_err(|_| denied())?;
        tokio::process::Command::new(executable()?)
            .arg("connect")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Never forward local diagnostics or terminal bytes into logs.
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?
    };
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("hc stdin unavailable"))?;
    let mut output = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("hc stdout unavailable"))?;
    let upload = async {
        let mut buffer = [0u8; 16384];
        loop {
            let Some(n) = recv.read(&mut buffer).await.map_err(io::Error::other)? else {
                break;
            };
            let _guard = security.guard(auth).await.map_err(|_| denied())?;
            // A stalled local core cannot hold the revocation fence indefinitely.
            tokio::time::timeout(Duration::from_secs(5), input.write_all(&buffer[..n]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "hc input stalled"))??;
        }
        Ok::<_, io::Error>(())
    };
    let download = async {
        let mut buffer = [0u8; 16384];
        loop {
            let n = output.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            let _guard = security.guard(auth).await.map_err(|_| denied())?;
            tokio::time::timeout(Duration::from_secs(5), send.write_all(&buffer[..n]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "phone output stalled"))?
                .map_err(io::Error::other)?;
        }
        Ok::<_, io::Error>(())
    };
    // Either half ending tears down the entire process/stream; no detached sessions or queues.
    let result = tokio::select! {
        result = upload => result,
        result = download => result,
        _ = connection.closed() => Ok(()),
        _ = connection.accept_bi() => Err(io::Error::other("unexpected bridge stream")),
        _ = connection.accept_uni() => Err(io::Error::other("unexpected bridge stream")),
    };
    let _ = child.start_kill();
    let _ = child.wait().await;
    connection.close(0u32.into(), b"bridge ended");
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn bridge_alpn_is_explicit_and_fail_closed() {
        assert!(crate::authorized_alpn(crate::HC_ALPN));
        assert!(crate::authorized_alpn(crate::ALPN));
        for other in [b"".as_slice(), b"goatr/hc/2", b"hc connect", b"goatr/1"] {
            assert!(!crate::authorized_alpn(other));
        }
    }
}
