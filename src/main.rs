use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::process;
use std::sync::mpsc::{channel};
use std::thread;
use std::time::{Duration, Instant};

use url::{Url};
use std::collections::HashMap;

// NodeConfig represents a parsed node configuration
#[derive(Clone)]
struct NodeConfig {
    protocol: String,
    address: String,
    port: u16,
    uuid: String,
    params: HashMap<String, String>,
    fragment: String, // Added to store the fragment (node name)
}

// Result represents a tested IP with its latency
#[derive(Clone)]
struct Result {
    ip: String,
    latency: Duration,
}

// parse_node_link parses a node link (e.g., vless://uuid@address:port?params#fragment)
fn parse_node_link(link: &str) -> std::result::Result<NodeConfig, Box<dyn std::error::Error>> {
    let u = Url::parse(link)?;

    let config = NodeConfig {
        protocol: u.scheme().to_string(),
        params: HashMap::new(),
        fragment: u.fragment().unwrap_or("").to_string(),
        address: String::new(),
        port: 0,
        uuid: String::new(),
    };

    let mut config = config;

    // Extract host and port
    let host = u.host_str().ok_or("Invalid host")?.to_string();
    let port = u.port().ok_or("Invalid port")?;

    config.address = host;
    config.port = port;

    // Extract UUID from userinfo
    config.uuid = u.username().to_string();

    // Parse query parameters
    for (key, value) in u.query_pairs() {
        config.params.insert(key.into_owned(), value.into_owned());
    }

    Ok(config)
}

// test_ip_latency tests the latency to a given IP and port
fn test_ip_latency(ip: &str, port: u16, timeout: Duration) -> std::result::Result<Duration, std::io::Error> {
    let start = Instant::now();
    let addr = format!("{}:{}", ip, port).to_socket_addrs()?.next().ok_or(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "Invalid address",
    ))?;
    let _conn = TcpStream::connect_timeout(&addr, timeout)?;
    Ok(start.elapsed())
}

// generate_node_link generates a new node link with the specified IP and preserves the fragment
fn generate_node_link(config: &NodeConfig, new_ip: &str) -> String {
    let mut query = String::new();
    let mut first = true;
    for (k, v) in &config.params {
        if !first {
            query.push('&');
        }
        query.push_str(&format!("{}={}", k, url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()));
        first = false;
    }

    let mut u = format!(
        "{}://{}@{}:{}",
        config.protocol, config.uuid, new_ip, config.port
    );
    if !query.is_empty() {
        u.push('?');
        u.push_str(&query);
    }
    if !config.fragment.is_empty() {
        u.push('#');
        u.push_str(&config.fragment);
    }
    u
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        println!("Usage: cdn_optimizer <node_link> <ip_list_file> [timeout_ms]");
        process::exit(1);
    }

    let node_link = &args[1];
    let ip_list_file = &args[2];
    let mut timeout = Duration::from_secs(3);
    if args.len() > 3 {
        let ms: u64 = args[3].parse().expect("Invalid timeout");
        timeout = Duration::from_millis(ms);
    }

    // Parse node link
    let config = match parse_node_link(node_link) {
        Ok(c) => c,
        Err(e) => {
            println!("Error parsing node link: {}", e);
            process::exit(1);
        }
    };

    // Read IP list from file
    let file = match File::open(ip_list_file) {
        Ok(f) => f,
        Err(e) => {
            println!("Error opening IP list file: {}", e);
            process::exit(1);
        }
    };
    let reader = io::BufReader::new(file);

    let mut ips: Vec<String> = Vec::new();
    for line in reader.lines() {
        let ip_str = line.unwrap().trim().to_string();
        if ip_str.parse::<IpAddr>().is_ok() {
            ips.push(ip_str);
        }
    }

    // Test IPs concurrently
    let (tx, rx) = channel();
    for ip in ips {
        let tx = tx.clone();
        let config = config.clone();
        thread::spawn(move || {
            if let Ok(latency) = test_ip_latency(&ip, config.port, timeout) {
                tx.send(Result { ip, latency }).unwrap();
            }
        });
    }

    drop(tx); // Close the channel after spawning all threads

    // Collect results
    let mut valid_results: Vec<Result> = rx.iter().collect();

    // Sort results by latency
    valid_results.sort_by_key(|r| r.latency);

    // Generate output file
    let output_file = match File::create("optimized_nodes.txt") {
        Ok(f) => f,
        Err(e) => {
            println!("Error creating output file: {}", e);
            process::exit(1);
        }
    };
    let mut output = BufWriter::new(output_file);

    // Write optimized node links (top 10 or all if fewer)
    let count = std::cmp::min(10, valid_results.len());
    for i in 0..count {
        let new_link = generate_node_link(&config, &valid_results[i].ip);
        let line = format!("{} (Latency: {:?})\n", new_link, valid_results[i].latency);
        output.write_all(line.as_bytes()).unwrap();
        print!("{}", line);
    }

    println!("Generated {} optimized node links in optimized_nodes.txt", count);
}