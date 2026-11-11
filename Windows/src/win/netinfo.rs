//! Lists the PC's network adapters with GetAdaptersAddresses; `airwand::lan::pick` chooses the usable ones.
use airwand::lan::{pick, Adapter};
use std::net::Ipv4Addr;
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

/// Addresses a phone on the same Wi-Fi or Ethernet network can reach (VPN and virtual adapters left out).
pub fn lan_ipv4s() -> Vec<Ipv4Addr> {
    pick(&adapters())
}

fn adapters() -> Vec<Adapter> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut words: usize = 4096; // u64 words, so the buffer is aligned for the structs
    for _ in 0..4 {
        let mut buf = vec![0u64; words];
        let mut bytes = (buf.len() * 8) as u32;
        let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let r = unsafe { GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(first), &mut bytes) };
        if r == 0 {
            return walk(first);
        }
        if r == 111 {
            // ERROR_BUFFER_OVERFLOW: `bytes` now holds the size needed
            words = bytes as usize / 8 + 16;
            continue;
        }
        break;
    }
    Vec::new()
}

fn walk(first: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<Adapter> {
    let mut out = Vec::new();
    let mut a = first;
    while !a.is_null() {
        let ad = unsafe { &*a };
        let text = |p: windows::core::PWSTR| if p.is_null() { String::new() } else { unsafe { p.to_string().unwrap_or_default() } };
        let name = format!("{} {}", text(ad.FriendlyName), text(ad.Description));
        let mut ips = Vec::new();
        let mut u = ad.FirstUnicastAddress;
        while !u.is_null() {
            let ua = unsafe { &*u };
            let sa = ua.Address.lpSockaddr;
            if !sa.is_null() && unsafe { (*sa).sa_family } == AF_INET {
                let sin = unsafe { &*(sa as *const SOCKADDR_IN) };
                let o = unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes();
                ips.push(Ipv4Addr::new(o[0], o[1], o[2], o[3]));
            }
            u = ua.Next;
        }
        out.push(Adapter { name, up: ad.OperStatus.0 == 1, if_type: ad.IfType, has_gateway: !ad.FirstGatewayAddress.is_null(), ips });
        a = ad.Next;
    }
    out
}
