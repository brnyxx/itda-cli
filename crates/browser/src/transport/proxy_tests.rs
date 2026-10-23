use super::*;
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::mpsc,
};

fn connect(proxy: &GuardedProxy, host: &str, port: u16, command: u8) -> TcpStream {
    let mut client = TcpStream::connect(proxy.address()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.write_all(&[5, 1, 0]).unwrap();
    let mut reply = [0; 2];
    client.read_exact(&mut reply).unwrap();
    assert_eq!(reply, [5, 0]);
    let mut request = vec![5, command, 0, 3, u8::try_from(host.len()).unwrap()];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    client.write_all(&request).unwrap();
    client
}

#[test]
fn private_dns_and_non_connect_requests_are_denied() {
    let proxy = GuardedProxy::new().unwrap();
    proxy
        .activate(false, Instant::now() + Duration::from_secs(2))
        .unwrap();
    for (host, command) in [("localhost", 1), ("127.0.0.1", 1), ("example.com", 3)] {
        let mut client = connect(&proxy, host, 80, command);
        let mut reply = [0; 10];
        client.read_exact(&mut reply).unwrap();
        assert_ne!(reply[1], 0);
    }
}

#[test]
fn idle_closes_tunnels_and_does_not_reopen_old_generation() {
    let proxy = GuardedProxy::new().unwrap();
    proxy
        .activate(true, Instant::now() + Duration::from_secs(5))
        .unwrap();
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = upstream.local_addr().unwrap().port();
    let (sent, received) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = upstream.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut data = [0; 4];
        stream.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"ping");
        stream.write_all(b"pong").unwrap();
        let n = stream.read(&mut data).unwrap();
        sent.send(n).unwrap();
    });
    let mut client = connect(&proxy, "127.0.0.1", port, 1);
    let mut reply = [0; 10];
    client.read_exact(&mut reply).unwrap();
    assert_eq!(reply[1], 0);
    client.write_all(b"ping").unwrap();
    let mut pong = [0; 4];
    client.read_exact(&mut pong).unwrap();
    assert_eq!(&pong, b"pong");
    proxy.deactivate().unwrap();
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
    assert_eq!(client.read(&mut pong).unwrap(), 0);
    let mut idle = TcpStream::connect(proxy.address()).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(idle.read(&mut pong).unwrap(), 0);
    proxy
        .activate(true, Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(client.read(&mut pong).unwrap(), 0);
    thread.join().unwrap();
}

#[test]
fn expired_action_rejects_new_connections() {
    let proxy = GuardedProxy::new().unwrap();
    proxy.activate(true, Instant::now()).unwrap();
    let mut client = TcpStream::connect(proxy.address()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(client.read(&mut [0]).unwrap(), 0);
}
