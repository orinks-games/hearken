//! Temporary: certificate checks against Freight Fate's HTTPS stations,
//! ureq's own check against the platform verifier. Removed after the run.
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn platform() -> ureq::Agent {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    agent(ureq::AgentBuilder::new().tls_config(Arc::new(config)))
}

fn agent(builder: ureq::AgentBuilder) -> ureq::Agent {
    builder
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(8))
        .build()
}

fn tls_error(agent: &ureq::Agent, url: &str) -> Option<String> {
    match agent.get(url).set("Icy-MetaData", "1").call() {
        Ok(_) | Err(ureq::Error::Status(..)) => None,
        Err(ureq::Error::Transport(t)) => {
            let text = format!("{t}");
            let tls = [
                "certificate",
                "Certificate",
                "UnknownIssuer",
                "NotValidForName",
                "Expired",
            ];
            tls.iter().any(|k| text.contains(k)).then_some(text)
        }
    }
}

#[test]
#[ignore]
fn tls_probe() {
    let urls: Vec<String> = std::fs::read_to_string(std::env::var("URLS_FILE").unwrap())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let queue = Arc::new(Mutex::new(urls));
    let results = Arc::new(Mutex::new(Vec::new()));
    let workers: Vec<_> = (0..48)
        .map(|_| {
            let (queue, results) = (queue.clone(), results.clone());
            std::thread::spawn(move || {
                let (plain, platform) = (agent(ureq::AgentBuilder::new()), platform());
                loop {
                    let Some(url) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let before = tls_error(&plain, &url);
                    if before.is_none() {
                        continue;
                    }
                    let after = tls_error(&platform, &url);
                    results.lock().unwrap().push((url, before, after));
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let results = results.lock().unwrap();
    let fixed = results.iter().filter(|r| r.2.is_none()).count();
    for (url, before, after) in results.iter() {
        eprintln!("PROBE {url}\n  ureq: {before:?}\n  platform: {after:?}");
    }
    eprintln!(
        "SUMMARY failing with ureq's check: {}, fixed by the platform verifier: {fixed}",
        results.len()
    );
}
