//! Whether a server answers on an address: for `healthcheck`, and for `install` and `serve --open`,
//! which find the one already running.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use anyhow::Result;

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "health-error")]
pub(crate) enum Error {
    NotAnswering {
        listen: String,
        source: std::io::Error,
    },
    #[fluent(answer = format!("{answer:?}"))]
    Unhealthy {
        answer: String,
    },
}

pub(crate) fn check(listen: &str) -> Result<()> {
    use std::io::{Read, Write};
    let ask = || -> std::io::Result<String> {
        let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(listen)?.map(loopback).collect();
        let mut stream = std::net::TcpStream::connect(addrs.as_slice())?;
        stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    };
    let response = ask().map_err(|source| Error::NotAnswering { listen: listen.to_owned(), source })?;
    let status = response.lines().next().unwrap_or_default();
    if status.split(' ').nth(1) != Some("200") {
        return Err(Error::Unhealthy { answer: status.to_owned() }.into());
    }
    Ok(())
}

/// A server listening on every address answers this computer on loopback, which Windows requires: it
/// refuses to connect to an unspecified address.
pub(crate) fn loopback(mut addr: SocketAddr) -> SocketAddr {
    if addr.ip().is_unspecified() {
        addr.set_ip(if addr.is_ipv4() { Ipv4Addr::LOCALHOST.into() } else { Ipv6Addr::LOCALHOST.into() });
    }
    addr
}
