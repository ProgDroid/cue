use cue::config::Config;

fn main() {
    let cfg = Config::from_env();
    println!("cue config loaded: bind={}", cfg.bind_addr);
}
