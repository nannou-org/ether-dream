extern crate ether_dream;

fn main() {
    let dac_broadcasts = ether_dream::recv_dac_broadcasts().expect("failed to bind to UDP socket");
    for dac_broadcast in dac_broadcasts {
        println!("{:#?}", dac_broadcast);
    }
}
