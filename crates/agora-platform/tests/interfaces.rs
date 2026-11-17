//! `interface_addresses`: the system lists this machine's addresses (Linux, macOS and Windows alike).

#[test]
fn the_machines_addresses_are_listed_or_the_refusal_says_why() {
    match agora_platform::interface_addresses() {
        Ok(list) => {
            println!("{} interface addresses", list.len());
            for a in &list {
                println!(
                    "  {} {} /{} up={} loopback={}",
                    a.name, a.ip, a.prefix_len, a.up, a.loopback
                );
                let max = if a.ip.is_ipv4() { 32 } else { 128 };
                assert!(a.prefix_len <= max, "{a:?}");
            }
            // A machine with a loopback interface (every Unix; Windows lists its loopback adapter as well) shows it.
            if cfg!(unix) {
                assert!(
                    list.iter().any(|a| a.loopback && a.ip.is_loopback()),
                    "no loopback address in {list:?}"
                );
            }
            if list.is_empty() {
                println!(
                    "empty: the system listed no unicast addresses (no adapter is configured)"
                );
            }
        }
        Err(e) => panic!("the system refused to list its interfaces: {e}"),
    }
}
