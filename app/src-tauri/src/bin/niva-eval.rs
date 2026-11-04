//! `niva-eval`: the E1 tool-choice evaluation (task 2.10). Built only with
//! `--features eval`; see `niva_lib::eval::cli` for the options.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(niva_lib::eval::cli::run(args));
}
