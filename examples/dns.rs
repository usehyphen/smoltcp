#[macro_use]
extern crate log;
extern crate byteorder;
extern crate env_logger;
extern crate getopts;
extern crate smoltcp;

mod utils;

use smoltcp::iface::{InterfaceBuilder, NeighborCache, Routes, SocketSet};
use smoltcp::phy::Device;
use smoltcp::phy::{wait as phy_wait, Medium};
use smoltcp::socket::dns::{self, GetQueryResultError};
use smoltcp::time::Instant;
use smoltcp::wire::{
    DnsQueryType, EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address, Ipv6Address,
};
use std::os::unix::io::AsRawFd;

fn main() {
    utils::setup_logging("warn");

    let (mut opts, mut free) = utils::create_options();
    utils::add_tuntap_options(&mut opts, &mut free);
    utils::add_middleware_options(&mut opts, &mut free);
    free.push("ADDRESS");

    let mut matches = utils::parse_options(&opts, free);
    let device = utils::parse_tuntap_options(&mut matches);
    let fd = device.as_raw_fd();
    let mut device =
        utils::parse_middleware_options(&mut matches, device, /*loopback=*/ false);
    let name = &matches.free[0];

    let neighbor_cache = NeighborCache::new();

    let servers = &[
        Ipv4Address::new(8, 8, 4, 4).into(),
        Ipv4Address::new(8, 8, 8, 8).into(),
    ];
    let dns_socket = dns::Socket::new(servers, vec![]);

    let ethernet_addr = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
    let src_ipv6 = IpAddress::v6(0xfdaa, 0, 0, 0, 0, 0, 0, 1);
    let mut ip_addrs = heapless::Vec::<IpCidr, 5>::new();
    ip_addrs
        .push(IpCidr::new(IpAddress::v4(192, 168, 69, 1), 24))
        .unwrap();
    ip_addrs.push(IpCidr::new(src_ipv6, 64)).unwrap();
    ip_addrs
        .push(IpCidr::new(IpAddress::v6(0xfe80, 0, 0, 0, 0, 0, 0, 1), 64))
        .unwrap();
    let default_v4_gw = Ipv4Address::new(192, 168, 69, 100);
    let default_v6_gw = Ipv6Address::new(0xfe80, 0, 0, 0, 0, 0, 0, 0x100);
    let mut routes = Routes::new();
    routes.add_default_ipv4_route(default_v4_gw).unwrap();
    routes.add_default_ipv6_route(default_v6_gw).unwrap();

    let medium = device.capabilities().medium;
    let mut builder = InterfaceBuilder::new().ip_addrs(ip_addrs).routes(routes);
    if medium == Medium::Ethernet {
        builder = builder
            .hardware_addr(HardwareAddress::Ethernet(ethernet_addr))
            .neighbor_cache(neighbor_cache);
    }
    let mut iface = builder.finalize(&mut device);

    let mut sockets = SocketSet::new(vec![]);
    let dns_handle = sockets.add(dns_socket);

    let socket = sockets.get_mut::<dns::Socket>(dns_handle);
    let query = socket
        .start_query(iface.context(), name, DnsQueryType::A)
        .unwrap();

    loop {
        let timestamp = Instant::now();
        debug!("timestamp {:?}", timestamp);

        match iface.poll(timestamp, &mut device, &mut sockets) {
            Ok(_) => {}
            Err(e) => {
                debug!("poll error: {}", e);
            }
        }

        match sockets
            .get_mut::<dns::Socket>(dns_handle)
            .get_query_result(query)
        {
            Ok(addrs) => {
                println!("Query done: {addrs:?}");
                break;
            }
            Err(GetQueryResultError::Pending) => {} // not done yet
            Err(e) => panic!("query failed: {e:?}"),
        }

        phy_wait(fd, iface.poll_delay(timestamp, &sockets)).expect("wait error");
    }
}
