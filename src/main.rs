extern crate pnet_packet;
extern crate pnet_base;
extern crate pcap;
#[macro_use] extern crate log;
extern crate env_logger;
extern crate failure;

use std::str::FromStr;
use std::net::Ipv6Addr;

use failure::Error;
use pnet_base::MacAddr;
use pnet_packet::icmpv6::*;
use pnet_packet::ipv6::*;
use pnet_packet::ethernet::*;
use pnet_packet::ip::IpNextHeaderProtocol;
use pnet_packet::Packet;
use pcap::{Device, Capture, Direction};

const MAX_PACKET_SIZE: usize = 1024;
const BPF_FILTER: &'static str = "icmp6";

fn main() -> Result<(), Error> {
    env_logger::init();

    let device = Device::lookup()?;
    let mut inactive_socket = Capture::from_device(device)?;
    inactive_socket = inactive_socket
                        .snaplen(MAX_PACKET_SIZE as i32);
    let mut sock = inactive_socket.open()?;
    sock.filter(BPF_FILTER)?;
    sock.direction(Direction::In)?;

    info!("socket active, listening");

    loop {
        let packet = sock.next()?;
        let ethernet_packet = EthernetPacket::new(&packet.data.owned());
        if ethernet_packet.is_none() {
            warn!("Couldn't read Ethernet packet??");
            continue;
        }

        let ethernet_packet = ethernet_packet.unwrap();

        let ip_packet = Ipv6Packet::new(&ethernet_packet.payload());
        if ip_packet.is_none() {
            warn!("Couldn't read IP packet??");
            warn!("Found ethernet packet {:?}", ethernet_packet);
            continue;
        }

        let ip_packet = ip_packet.unwrap();

        let icmp_packet = Icmpv6Packet::new(&ip_packet.payload());
        if icmp_packet.is_none() {
            warn!("Couldn't read ICMP packet??");
            warn!("Found IPv6 packet {:?}", ip_packet);
            continue;
        }

        let icmp_packet = icmp_packet.unwrap();

        if icmp_packet.get_icmpv6_type() != Icmpv6Types::EchoRequest {
            continue;
        }

        info!("ICMP Echo Request from {:?} hop limit {:?}", ip_packet.get_source(), ip_packet.get_hop_limit());
        debug!("Read ethernet packet {:?}", ethernet_packet);
        debug!("Read IPv6 packet {:?}", ip_packet);
        debug!("Read ICMP {:?} / type: {:?}", icmp_packet, icmp_packet.get_icmpv6_type());

        let ips = vec![
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:0").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:1").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:2").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:3").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:4").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:5").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:6").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:7").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:8").unwrap(),
            Ipv6Addr::from_str("2605:2700:1:1019:0:0:0:9").unwrap(),
        ];

        let resp_packet = create_reply(&ips, &ip_packet);
        debug!("Sending resp: {:?}", resp_packet);

        sock.sendpacket(resp_packet).unwrap();
    }
}

fn create_icmp_time_exceeded(source: &Ipv6Addr, prev_packet: &Ipv6Packet) -> Vec<u8> {
    let mut icmp_buf = [0u8; MAX_PACKET_SIZE - 40]; // 40 comes from the IPv6 header size
    let mut icmp = MutableIcmpv6Packet::new(&mut icmp_buf).unwrap();
    icmp.set_icmpv6_type(Icmpv6Types::TimeExceeded);
    icmp.set_icmpv6_code(Icmpv6Code::new(0));

    let mut payload = vec![];
    // blank area
    payload.extend_from_slice(&[0; 4]);
    // prev packet
    payload.extend_from_slice(&prev_packet.packet()[0..(40 + 8 + 8)]);

    icmp.set_payload(&payload);
    let icmp_packet_size = payload.len() + MutableIcmpv6Packet::minimum_packet_size();

    let just_bloody_copy_it = Icmpv6Packet::owned(icmp.packet()[0..icmp_packet_size].to_owned()).unwrap();
    icmp.set_checksum(checksum(&just_bloody_copy_it, source, &prev_packet.get_source()));

    Vec::from(&icmp.packet()[0..icmp_packet_size])
}

fn create_icmp_echo_reply(prev_packet: &Ipv6Packet) -> Vec<u8> {
    let mut icmp_buf = [0u8; MAX_PACKET_SIZE - 40]; // 40 comes from the IPv6 header size
    let mut icmp = MutableIcmpv6Packet::new(&mut icmp_buf).unwrap();
    icmp.set_icmpv6_type(Icmpv6Types::EchoReply);
    icmp.set_icmpv6_code(Icmpv6Code::new(0));

    let payload = &prev_packet.payload()[4..16];
    icmp.set_payload(&payload);
    let icmp_packet_size = payload.len() + MutableIcmpv6Packet::minimum_packet_size();

    let just_bloody_copy_it = Icmpv6Packet::owned(icmp.packet()[0..icmp_packet_size].to_owned()).unwrap();
    icmp.set_checksum(checksum(&just_bloody_copy_it, &prev_packet.get_destination(), &prev_packet.get_source()));

    Vec::from(&icmp.packet()[0..icmp_packet_size])
}

fn create_ip_reply(ips: &[Ipv6Addr], prev_packet: &Ipv6Packet) -> Vec<u8> {
    let mut buf = [0u8; MAX_PACKET_SIZE];
    let mut ipv6_header = MutableIpv6Packet::new(&mut buf).unwrap();

    ipv6_header.set_destination(prev_packet.get_source());
    ipv6_header.set_version(6 as u8);
    ipv6_header.set_next_header(IpNextHeaderProtocol(58)); // ICMP
    ipv6_header.set_hop_limit(64);

    let (icmp, source) = match ips.get(prev_packet.get_hop_limit() as usize) {
        // send an IP we own
        Some(source) => (create_icmp_time_exceeded(source, prev_packet), source.to_owned()),
        // we've run out of things to say, time to send the echo reply
        None => (create_icmp_echo_reply(prev_packet), prev_packet.get_destination()),
    };

    ipv6_header.set_source(source);

    let icmp_packet_size = icmp.len();
    ipv6_header.set_payload_length(icmp_packet_size as u16); // FIXME(nickhs): is this safe
    ipv6_header.set_payload(&icmp);

    let packet_size = MutableIpv6Packet::minimum_packet_size() + icmp_packet_size;
    debug!("Returning {:?} / {:?}", ipv6_header, icmp);
    return Vec::from(&ipv6_header.packet()[0..packet_size]);
}

fn make_ethernet<'b>() -> MutableEthernetPacket<'b> {
    let buf = [0u8; MAX_PACKET_SIZE];
    let mut ethernet_reply = MutableEthernetPacket::owned(buf.to_vec()).unwrap();

    // 00:60:dd:46:62:3d
    ethernet_reply.set_destination(MacAddr::new(0, 96, 221, 70, 98, 61));

    // aa:00:00:76:0d:28
    ethernet_reply.set_source(MacAddr::new(170, 0, 0, 118, 13, 40));

    ethernet_reply.set_ethertype(EtherTypes::Ipv6);
    ethernet_reply
}

fn create_reply(ips: &[Ipv6Addr], prev_packet: &Ipv6Packet) -> Vec<u8> {
    let mut eth1 = make_ethernet();

    let payload = create_ip_reply(ips, prev_packet);
    eth1.set_payload(&payload);

    let packet_size = MutableEthernetPacket::minimum_packet_size() + payload.len();
    debug!("Returning {:?}", eth1);
    return Vec::from(&eth1.packet()[0..packet_size]);
}
