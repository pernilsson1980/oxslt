fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let code = oxslt::run_before_exit(
        &args,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
