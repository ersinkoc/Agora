fn main() {
    let mut args = std::env::args().skip(1);
    let title = args.next().unwrap();
    let body = args.next().unwrap();
    match agora_platform::desktop_notify(&title, &body) {
        Ok(()) => println!("ok"),
        Err(e) => println!("error:{:?}", e.kind()),
    }
}
