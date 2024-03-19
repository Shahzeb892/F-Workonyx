//! Spray system binary

use clap::Parser;
use onyx::components::prelude::*;

/// Arguments required for starting the program from the command line.
#[derive(Parser, Debug)]
struct Args {
    /// Path to the config file for the Crop Bed Power Component.
    #[arg(short, long)]
    filepath: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let component = CropBedPower::from_config_file(args.filepath);
    CropBedPowerController::start(component).await;
}

