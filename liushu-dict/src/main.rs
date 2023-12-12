use clap::{Parser, Subcommand};
use liushu_core::dict::build2;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Build dictionary
    Build {
        #[arg(short, long)]
        inputs: Vec<String>,

        #[arg(short, long)]
        output_dir: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Some(Commands::Build { inputs, output_dir }) => {
            build2(inputs, output_dir).unwrap();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::Cli;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert()
    }
}
