fn main() {
    if let Err(error) = cyber_agent::run(std::env::args_os()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
