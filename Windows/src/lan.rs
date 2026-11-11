//! Which of this PC's addresses a phone on the same Wi-Fi can actually reach.
//! The default-route trick picks a VPN tunnel address (such as 10.2.0.2) while a VPN is on, and a phone
//! cannot reach that. So the real network adapters are listed and the virtual ones are skipped.
use std::net::Ipv4Addr;

#[derive(Debug, Clone)]
pub struct Adapter {
    /// Friendly name and description, joined.
    pub name: String,
    pub up: bool,
    /// IANA interface type: 6 Ethernet, 71 Wi-Fi.
    pub if_type: u32,
    pub has_gateway: bool,
    pub ips: Vec<Ipv4Addr>,
}

const VIRTUAL: [&str; 24] = [
    "vpn", "tap", "tun", "wintun", "wireguard", "proton", "nord", "mullvad", "virtual", "vmware", "vbox", "hyper-v", "vethernet", "wsl", "docker",
    "tailscale", "zerotier", "loopback", "bluetooth", "pseudo", "npcap", "cisco", "fortinet", "openvpn",
];

/// Private (home or office) addresses of up Ethernet and Wi-Fi adapters, adapters with a gateway first.
pub fn pick(adapters: &[Adapter]) -> Vec<Ipv4Addr> {
    let mut good: Vec<&Adapter> = adapters
        .iter()
        .filter(|a| a.up && (a.if_type == 6 || a.if_type == 71))
        .filter(|a| {
            let n = a.name.to_ascii_lowercase();
            !VIRTUAL.iter().any(|v| n.contains(v))
        })
        .collect();
    good.sort_by_key(|a| !a.has_gateway); // stable: keeps the OS order otherwise
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for a in good {
        for ip in &a.ips {
            if ip.is_private() && !out.contains(ip) {
                out.push(*ip);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(name: &str, t: u32, gw: bool, ip: [u8; 4]) -> Adapter {
        Adapter { name: name.into(), up: true, if_type: t, has_gateway: gw, ips: vec![Ipv4Addr::from(ip)] }
    }

    #[test]
    fn vpn_and_virtual_adapters_are_skipped() {
        let list = vec![
            ad("ProtonVPN WireGuard", 53, true, [10, 2, 0, 2]),
            ad("Wi-Fi Intel(R) Wi-Fi 6 AX201", 71, true, [192, 168, 29, 7]),
            ad("vEthernet (WSL) Hyper-V Virtual Ethernet Adapter", 6, false, [172, 20, 0, 1]),
            ad("Ethernet VirtualBox Host-Only Ethernet Adapter", 6, false, [192, 168, 56, 1]),
        ];
        assert_eq!(pick(&list), vec![Ipv4Addr::new(192, 168, 29, 7)]);
    }

    #[test]
    fn a_vpn_that_poses_as_ethernet_is_still_skipped_by_name() {
        let list = vec![ad("Ethernet 3 TAP-Windows Adapter V9", 6, true, [10, 8, 0, 6]), ad("Ethernet Realtek PCIe", 6, true, [10, 0, 0, 12])];
        assert_eq!(pick(&list), vec![Ipv4Addr::new(10, 0, 0, 12)]);
    }

    #[test]
    fn gateway_first_and_only_private_addresses() {
        let mut down = ad("Wi-Fi", 71, true, [192, 168, 1, 5]);
        down.up = false;
        let list = vec![ad("Ethernet", 6, false, [192, 168, 1, 9]), ad("Wi-Fi", 71, true, [192, 168, 1, 8]), down, ad("Ethernet 2", 6, true, [169, 254, 3, 3]), ad("Ethernet 4", 6, true, [8, 8, 4, 4])];
        assert_eq!(pick(&list), vec![Ipv4Addr::new(192, 168, 1, 8), Ipv4Addr::new(192, 168, 1, 9)]);
        assert!(pick(&[]).is_empty());
    }
}
