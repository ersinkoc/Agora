// Append to lib.rs for a review-only measurement; baseline module is the original
// b2fe875 router.rs, without its test module. Both paths run in the same binary.
#[cfg(test)]
mod review_574_paired {
    macro_rules! measure {
        ($router:ty, $clients:expr, $size:expr) => {{
            let router = <$router>::new();
            let (_registration, _node_receiver) = router.register("n");
            let mut windows = Vec::new();
            for c in 0..$clients {
                let (client, frames) = router.client();
                let stream = router.open_relay("n", &client, 7, Some(&format!("t-{c}")));
                windows.push((client, frames, stream));
            }
            let rounds = 100_000 / $clients;
            let start = std::time::Instant::now();
            for i in 0..rounds {
                for (_, _, stream) in &windows {
                    router.relay_frame("n", *stream, 0, vec![0u8; $size]);
                }
                if i % 256 == 255 {
                    for (_, frames, _) in &mut windows {
                        while frames.try_recv().is_ok() {}
                    }
                }
            }
            let ns = start.elapsed().as_nanos() as f64 / (rounds * $clients) as f64;
            for (client, _, _) in &windows {
                assert!(
                    client.take_closed().is_empty(),
                    "measurement dropped a relay"
                );
            }
            ns
        }};
    }

    #[test]
    #[ignore = "measurement"]
    fn paired_relay_cost() {
        for (clients, size) in [(1usize, 64usize), (1, 4096), (8, 64), (8, 4096), (32, 4096)] {
            for sample in 0..9 {
                let (before, after) = if sample % 2 == 0 {
                    let before = measure!(crate::review_574_baseline::Router, clients, size);
                    (before, measure!(crate::router::Router, clients, size))
                } else {
                    let after = measure!(crate::router::Router, clients, size);
                    (
                        measure!(crate::review_574_baseline::Router, clients, size),
                        after,
                    )
                };
                println!("paired clients={clients} payload={size} sample={sample} before={before:.1} after={after:.1}");
            }
        }
    }
}
