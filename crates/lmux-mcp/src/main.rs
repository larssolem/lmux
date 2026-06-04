fn main() -> std::process::ExitCode {
    match lmux_mcp::run_stdio(std::io::stdin().lock(), std::io::stdout()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lmux-mcp: {err}");
            std::process::ExitCode::from(1)
        }
    }
}
