fn main() -> anyhow::Result<()> {
    lmux_macos_helper::run_stdio(std::io::stdin(), std::io::stdout())
}
