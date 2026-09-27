fn main() {
    if let Err(error) = echidna::run(std::env::args_os()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
