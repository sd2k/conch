use std::io::Read;

/// Upper-cases stdin, or probes what a spawned child can reach:
/// `env NAME` prints the variable (or `<unset>`), `net HOST:PORT` tries to
/// connect and prints `connected` or `blocked`.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1..).unwrap_or_default() {
        [mode, name] if mode == "env" => {
            println!(
                "{}",
                std::env::var(name).unwrap_or_else(|_| "<unset>".into())
            );
        }
        [mode, addr] if mode == "net" => match std::net::TcpStream::connect(addr) {
            Ok(_) => println!("connected"),
            Err(_) => println!("blocked"),
        },
        _ => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            print!("{}", input.to_uppercase());
        }
    }
}
