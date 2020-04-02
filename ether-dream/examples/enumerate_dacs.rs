//! Searches for all broadcasting DACs on the network for 3 seconds and then prints them to stdout.

extern crate ether_dream;

use ether_dream::dac;
use std::collections::HashMap;
use std::{io, time};

fn main() {
    println!("Searching for Ether Dream DACs...");

    let mut dacs = HashMap::new();
    let three_secs = time::Duration::from_secs(3);
    let mut rx = ether_dream::recv_dac_broadcasts().expect("failed to bind to UDP socket");
    rx.set_timeout(Some(three_secs))
        .expect("failed to set timeout on UDP socket");
    let loop_start = time::Instant::now();
    while loop_start.elapsed() < three_secs {
        let (dac_broadcast, source_addr) = match rx.next_broadcast() {
            Ok(dac) => dac,
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => continue,
                _ => panic!("an IO error occurred: {}", e),
            },
        };
        let dac::Addressed { mac_address, dac } = dac::Addressed::from_broadcast(&dac_broadcast)
            .expect("failed to interpret DAC status from received broadcast");
        if dacs.insert(mac_address, (dac, source_addr)).is_none() {
            println!("Discovered new DAC \"{}\"...", mac_address);
        }
    }

    if dacs.is_empty() {
        println!("No Ether Dream DACs found on the network.");
    } else {
        println!("Discovered the following Ether Dream DACs on the network:");
        for (i, (mac, (dac, source_addr))) in dacs.into_iter().enumerate() {
            println!(
                "{}.\n  MAC address: \"{}\"\n  Network address: \"{}\"\n  {:?}",
                i + 1,
                mac,
                source_addr,
                dac
            );
        }
    }
}
