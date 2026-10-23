use super::protocol::remaining;
use itda_types::error::{AppError, AppResult};
use std::{
    net::{IpAddr, ToSocketAddrs},
    sync::mpsc,
    time::Instant,
};
use url::Url;

pub(super) fn resolve(url: &Url, allow_local: bool, deadline: Instant) -> AppResult<Vec<IpAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| AppError::api("bad_url", 400))?
        .trim_matches(['[', ']'])
        .to_owned();
    let addresses: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => {
            let port = url
                .port_or_known_default()
                .ok_or_else(|| AppError::api("bad_url", 400))?;
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let result = (host.as_str(), port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.map(|a| a.ip()).collect::<Vec<_>>());
                let _ = sender.send(result);
            });
            receiver
                .recv_timeout(remaining(deadline)?)
                .map_err(|_| AppError::api("timeout", 502))?
                .map_err(|_| AppError::api("request_failed", 502))?
        }
    };
    if addresses.is_empty() || (!allow_local && addresses.iter().any(|ip| !public_ip(*ip))) {
        return Err(AppError::api("unsafe_target", 400));
    }
    Ok(addresses)
}

pub(super) fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || s[0] & 0xe000 != 0x2000
                || (s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] == 0 || s[1] == 2))
                || s[0] == 0x2002)
        }
    }
}
