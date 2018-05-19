//! Searches for all broadcasting DACs on the network for 3 seconds and then prints them to stdout.

extern crate ether_dream;

use ether_dream::dac;
use std::collections::HashMap;
use std::{sync, thread, time};

fn main() {
    println!("Searching for Ether Dream DACs...");

    let (dac_tx, dac_rx) = sync::mpsc::channel();
    thread::spawn(move || {
        ether_dream::recv_dac_broadcasts()
            .expect("failed to bind to UDP socket")
            .filter_map(Result::ok)
            .for_each(|(dac_broadcast, source_addr)| {
                let dac = dac::Addressed::from_broadcast(&dac_broadcast)
                    .expect("failed to interpret DAC status from received broadcast");
                dac_tx.send((dac, source_addr)).unwrap();
            });
    });

    let mut dacs = HashMap::new();
    let loop_start = time::Instant::now();
    while time::Instant::now().duration_since(loop_start) < time::Duration::from_secs(3) {
        for (dac::Addressed { mac_address, dac }, source_addr) in dac_rx.try_iter() {
            if dacs.insert(mac_address, (dac, source_addr)).is_none() {
                println!("Discovered new DAC \"{}\"...", mac_address);
            }
        }
        thread::sleep(time::Duration::from_millis(100));
    }

    if dacs.is_empty() {
        println!("No Ether Dream DACs found on the network.");
    } else {
        println!("Discovered the following Ether Dream DACs on the network:");
        for (i, (mac, (dac, source_addr))) in dacs.into_iter().enumerate() {
            println!("{}.\n  MAC address: \"{}\"\n  Network address: \"{}\"\n  {:?}",
                     i+1, mac, source_addr, dac);
        }
    }
}
