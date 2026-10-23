use super::{guard::public_ip, protocol::unavailable};
use itda_types::error::AppResult;
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::JoinSet,
};

#[derive(Clone, Copy)]
struct Policy {
    allow_local: bool,
    deadline: Instant,
}

enum Control {
    Policy(Option<Policy>, mpsc::SyncSender<()>),
    Stop,
}

pub(super) struct GuardedProxy {
    address: SocketAddr,
    control: UnboundedSender<Control>,
    worker: Option<JoinHandle<()>>,
}

impl GuardedProxy {
    pub(super) fn new() -> AppResult<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (control, mut commands) = unbounded_channel();
        let worker = std::thread::spawn(move || {
            runtime.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                    return;
                };
                let mut policy: Option<Policy> = None;
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        biased;
                        command = commands.recv() => {
                            connections.abort_all();
                            while connections.join_next().await.is_some() {}
                            match command {
                                Some(Control::Policy(next, ack)) => {
                                    policy = next;
                                    let _ = ack.send(());
                                }
                                Some(Control::Stop) | None => break,
                            }
                        }
                        accepted = listener.accept() => {
                            let Ok((socket, _)) = accepted else { break };
                            if let Some(policy) = policy
                                && policy.deadline > Instant::now()
                                && connections.len() < 64
                            {
                                connections.spawn(async move {
                                    let _ = tokio::time::timeout_at(
                                        policy.deadline.into(),
                                        tunnel(socket, policy),
                                    ).await;
                                });
                            }
                        }
                        _ = connections.join_next(), if !connections.is_empty() => {}
                    }
                }
            });
            runtime.shutdown_background();
        });
        Ok(Self {
            address,
            control,
            worker: Some(worker),
        })
    }

    pub(super) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(super) fn activate(&self, allow_local: bool, deadline: Instant) -> AppResult<()> {
        self.set_policy(Some(Policy {
            allow_local,
            deadline,
        }))
    }

    pub(super) fn deactivate(&self) -> AppResult<()> {
        self.set_policy(None)
    }

    fn set_policy(&self, policy: Option<Policy>) -> AppResult<()> {
        let (ack, received) = mpsc::sync_channel(1);
        self.control
            .send(Control::Policy(policy, ack))
            .map_err(|_| unavailable())?;
        received
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| unavailable())
    }
}

impl Drop for GuardedProxy {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn tunnel(mut client: TcpStream, policy: Policy) -> io::Result<()> {
    let mut greeting = [0; 2];
    client.read_exact(&mut greeting).await?;
    if greeting[0] != 5 || greeting[1] == 0 {
        return Ok(());
    }
    let mut methods = vec![0; usize::from(greeting[1])];
    client.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        client.write_all(&[5, 255]).await?;
        return Ok(());
    }
    client.write_all(&[5, 0]).await?;
    let mut request = [0; 4];
    client.read_exact(&mut request).await?;
    if request[..3] != [5, 1, 0] {
        client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    }
    let host = match request[3] {
        1 => {
            let mut bytes = [0; 4];
            client.read_exact(&mut bytes).await?;
            Ipv4Addr::from(bytes).to_string()
        }
        4 => {
            let mut bytes = [0; 16];
            client.read_exact(&mut bytes).await?;
            Ipv6Addr::from(bytes).to_string()
        }
        3 => {
            let length = client.read_u8().await?;
            let mut name = vec![0; usize::from(length)];
            client.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| io::ErrorKind::InvalidData)?
        }
        _ => {
            client.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };
    let port = client.read_u16().await?;
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host.as_str(), port))
            .await?
            .collect()
    };
    if addresses.is_empty()
        || (!policy.allow_local && addresses.iter().any(|address| !public_ip(address.ip())))
    {
        client.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    }
    let mut upstream = None;
    for address in addresses {
        if let Ok(socket) = TcpStream::connect(address).await {
            upstream = Some(socket);
            break;
        }
    }
    let Some(mut upstream) = upstream else {
        client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    };
    client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
