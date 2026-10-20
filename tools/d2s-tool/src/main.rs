//! `d2s-tool`: see [`d2s_tool::cli::USAGE`]. Exit 0 on success, 1 on an
//! error or a `check` difference, 2 on a usage error.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout();
    match d2s_tool::cli::run(&args, &mut out) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("d2s-tool: {e:#}");
            std::process::exit(1);
        }
    }
}
