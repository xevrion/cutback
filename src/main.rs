use clap::Parser;

fn main() {
    let cli = cutback::cli::Cli::parse();
    if let Err(e) = cutback::cli::run(cli) {
        eprintln!("cutback: {e}");
        // Anything that caused the failure is worth showing, since the useful
        // detail is usually in the source rather than the top level message.
        for cause in e.chain().skip(1) {
            eprintln!("  {cause}");
        }
        std::process::exit(1);
    }
}
