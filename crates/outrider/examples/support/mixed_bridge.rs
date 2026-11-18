//! Optional protected TCP connection to a Tulle bench bridge.
use retinue::{Ifac, endpoint::Endpoint};
use std::{error::Error, net::SocketAddr};

pub async fn attach_bridge(endpoint: &Endpoint) -> Result<bool, Box<dyn Error>> {
    let Some(address) = std::env::var_os("OUTRIDER_BRIDGE_ADDR") else {
        return Ok(false);
    };
    let address: SocketAddr = address.to_str().ok_or("invalid bridge address")?.parse()?;
    let pass = std::env::var("OUTRIDER_BRIDGE_PASSPHRASE")?;
    let ifac = Ifac::new(Some("signalman-qualification"), Some(&pass), 8)?;
    let mtu: u32 = std::env::var("OUTRIDER_LINK_MTU")?.parse()?;
    endpoint.set_link_mtu(mtu);
    endpoint.attach_tcp_client_with_ifac(address, ifac).await?;
    println!("PROTECTED_BRIDGE link_mtu={mtu}");
    Ok(true)
}
